/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/*
 * Leak loop of the C interface: every call a long-lived program makes, many times over, in
 * one process. Built and run like the other C tests (check.sh, capi-check.sh); CI also runs
 * it under clang's AddressSanitizer with LeakSanitizer (ASAN_OPTIONS=detect_leaks=1), which
 * fails the run on any block still allocated and unreachable at exit.
 *
 * Each iteration, for every rate, cutoff and filter (invalid combinations included), with
 * 1 to 6 channels: create a frame-run converter, convert in one buffer as packed 24-bit
 * and one buffer per frame as int32 with neighbours, check the layouts agree, hit the
 * error paths (NULLs, bad lengths and formats, overlapping buffers, too-small
 * struct_size), and free it. Then whole-file conversions, threaded and on one thread, with
 * a progress callback, cancelled before and after the first block, and refused (missing
 * input, a file that is not DSD, bad options, output equal to input). A thread that ends
 * after setting the last error checks the per-thread message is freed with the thread.
 *
 * On Linux without a sanitizer it also checks that the resident set size of the second
 * half of the loop stays within 5 % or 16 MiB of the highest value of the first half.
 *
 * Usage: leakloop [ITERATIONS [DIR]] (default 20 iterations; DIR as in smoke.c).
 */
#define _POSIX_C_SOURCE 200809L

#include "dsd2pcm.h"

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#if defined(__SANITIZE_ADDRESS__)
#define UNDER_ASAN 1
#elif defined(__has_feature)
#if __has_feature(address_sanitizer)
#define UNDER_ASAN 1
#endif
#endif

#define FRAMES 6
#define MAX_FRAME (DSD2PCM_MAX_CHANNELS * DSD2PCM_FRAME_BYTES_PER_CHANNEL)

static int failures;

#define CHECK(cond)                                                                        \
    do {                                                                                   \
        if (!(cond)) {                                                                     \
            fprintf(stderr, "%s:%d: check failed: %s (last error: %s)\n", __FILE__,        \
                    __LINE__, #cond, dsd2pcm_last_error());                                \
            if (++failures > 20) exit(1);                                                  \
        }                                                                                  \
    } while (0)

static void put32(unsigned char *p, uint32_t v) {
    for (int i = 0; i < 4; i++) p[i] = (unsigned char)(v >> (8 * i));
}

static void put64(unsigned char *p, uint64_t v) {
    for (int i = 0; i < 8; i++) p[i] = (unsigned char)(v >> (8 * i));
}

/* A DSF file of `frames` frames of stereo DSD64 (pseudo-random, LSB-first on disk). */
static int write_dsf(const char *path, size_t frames, uint64_t seed) {
    const size_t ch = 2, block = 4096;
    const size_t per_ch = frames * DSD2PCM_FRAME_BYTES_PER_CHANNEL;
    const size_t blocks = (per_ch + block - 1) / block;
    const size_t data = blocks * block * ch;
    unsigned char hdr[28 + 52 + 12] = {0};
    memcpy(hdr, "DSD ", 4);
    put64(hdr + 4, 28);
    put64(hdr + 12, sizeof hdr + data);
    memcpy(hdr + 28, "fmt ", 4);
    put64(hdr + 32, 52);
    put32(hdr + 40, 1);
    put32(hdr + 44, 0);
    put32(hdr + 48, 2);
    put32(hdr + 52, (uint32_t)ch);
    put32(hdr + 56, 2822400);
    put32(hdr + 60, 1);
    put64(hdr + 64, (uint64_t)per_ch * 8);
    put32(hdr + 72, (uint32_t)block);
    memcpy(hdr + 80, "data", 4);
    put64(hdr + 84, 12 + data);
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    fwrite(hdr, 1, sizeof hdr, f);
    unsigned char buf[4096];
    for (size_t i = 0; i < blocks * ch; i++) {
        for (size_t j = 0; j < block; j++) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            buf[j] = (unsigned char)seed;
        }
        fwrite(buf, 1, block, f);
    }
    return fclose(f);
}

struct progress {
    int calls;
    int stop_after;
    uint64_t last_done, total;
};

static int on_progress(uint64_t done, uint64_t total, void *user) {
    struct progress *p = user;
    p->calls++;
    p->last_done = done;
    p->total = total;
    return p->calls > p->stop_after;
}

/* KiB resident, from /proc/self/statm; 0 where there is none. */
static long rss_kib(void) {
    FILE *f = fopen("/proc/self/statm", "r");
    if (!f) return 0;
    long size = 0, res = 0;
    int n = fscanf(f, "%ld %ld", &size, &res);
    fclose(f);
    return n == 2 ? res * (sysconf(_SC_PAGESIZE) / 1024) : 0;
}

/* One converter's life: create, convert both layouts, error paths, free. */
static void frames_cycle(uint32_t rate, uint32_t cutoff, uint32_t filter, uint32_t ch,
                         double gain, const unsigned char *area) {
    dsd2pcm_frames_config cfg = DSD2PCM_FRAMES_CONFIG_INIT;
    cfg.rate_hz = rate;
    cfg.cutoff = cutoff;
    cfg.filter = filter;
    cfg.channels = ch;
    cfg.gain_db = gain;
    dsd2pcm_frames *conv = (dsd2pcm_frames *)&cfg; /* must be set to NULL on an error */
    int valid = !(cutoff == DSD2PCM_CUTOFF_WIDE && rate == 88200) &&
                !(filter == DSD2PCM_FILTER_PYSACD &&
                  (rate == 352800 || cutoff != DSD2PCM_CUTOFF_STANDARD));
    int rc = dsd2pcm_frames_new(&cfg, &conv);
    if (!valid) {
        CHECK(rc == DSD2PCM_E_INVALID_ARGUMENT && conv == NULL);
        CHECK(strlen(dsd2pcm_last_error()) > 0);
        return;
    }
    CHECK(rc == DSD2PCM_OK && conv != NULL);
    if (rc != DSD2PCM_OK) return;
    const size_t frame = dsd2pcm_frames_frame_bytes(conv);
    CHECK(frame == ch * DSD2PCM_FRAME_BYTES_PER_CHANNEL);
    CHECK(dsd2pcm_frames_context_bytes(conv) < DSD2PCM_FRAME_BYTES_PER_CHANNEL);
    const size_t spf = dsd2pcm_frames_samples_per_frame(conv);

    /* The whole area, one buffer, packed 24-bit, with per-frame clip counts. */
    size_t n24 = dsd2pcm_frames_output_bytes(conv, FRAMES, DSD2PCM_FORMAT_S24LE);
    unsigned char *pcm = malloc(n24);
    uint64_t fc[FRAMES];
    dsd2pcm_stats st = DSD2PCM_STATS_INIT;
    dsd2pcm_run run = DSD2PCM_RUN_INIT;
    run.frames = area;
    run.frames_len = FRAMES * frame;
    CHECK(dsd2pcm_frames_convert(conv, &run, DSD2PCM_FORMAT_S24LE, pcm, n24, fc, FRAMES,
                                 &st) == DSD2PCM_OK);
    uint64_t sum = 0;
    for (int i = 0; i < FRAMES; i++) sum += fc[i];
    CHECK(sum == st.clipped_total && st.channels == ch);

    /* Frames 1..4, one buffer each, int32, with neighbours: the same samples. */
    const unsigned char *ptrs[4];
    for (int i = 0; i < 4; i++) ptrs[i] = area + (size_t)(1 + i) * frame;
    dsd2pcm_run part = DSD2PCM_RUN_INIT;
    part.before = area;
    part.before_len = frame;
    part.frame_ptrs = ptrs;
    part.frame_count = 4;
    part.after = area + 5 * frame;
    part.after_len = frame;
    part.first_frame = 1;
    size_t n32 = dsd2pcm_frames_output_bytes(conv, 4, DSD2PCM_FORMAT_I32);
    int32_t *pcm32 = malloc(n32);
    CHECK(dsd2pcm_frames_convert(conv, &part, DSD2PCM_FORMAT_I32, pcm32, n32, NULL, 0, NULL) ==
          DSD2PCM_OK);
    int same = 1;
    for (size_t i = 0; i < n32 / 4; i++) {
        const unsigned char *b = pcm + 3 * (spf * ch + i);
        int32_t v =
            (int32_t)((uint32_t)b[0] << 8 | (uint32_t)b[1] << 16 | (uint32_t)b[2] << 24) >> 8;
        same &= v == pcm32[i];
    }
    CHECK(same);

    /* Error paths on a live handle. */
    CHECK(dsd2pcm_frames_convert(conv, NULL, 0, pcm, n24, NULL, 0, NULL) == DSD2PCM_E_NULL);
    CHECK(dsd2pcm_frames_convert(conv, &run, 0, NULL, n24, NULL, 0, NULL) == DSD2PCM_E_NULL);
    CHECK(dsd2pcm_frames_convert(conv, &run, 0, pcm, n24 - 1, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    CHECK(dsd2pcm_frames_convert(conv, &run, 7, pcm, n24, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    CHECK(dsd2pcm_frames_convert(conv, &run, 0, pcm, n24, fc, 2, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_run overlap = DSD2PCM_RUN_INIT;
    overlap.frames = pcm;
    overlap.frames_len = n24 >= frame ? frame : 0;
    if (overlap.frames_len)
        CHECK(dsd2pcm_frames_convert(conv, &overlap, 0, pcm + 1,
                                     dsd2pcm_frames_output_bytes(conv, 1, 0), NULL, 0,
                                     NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_run small = run;
    small.struct_size = 8;
    CHECK(dsd2pcm_frames_convert(conv, &small, 0, pcm, n24, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_run both = run;
    both.frame_ptrs = ptrs;
    both.frame_count = 4;
    CHECK(dsd2pcm_frames_convert(conv, &both, 0, pcm, n24, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    /* Still usable after the errors. */
    CHECK(dsd2pcm_frames_convert(conv, &run, DSD2PCM_FORMAT_S24LE, pcm, n24, NULL, 0, NULL) ==
          DSD2PCM_OK);

    dsd2pcm_frames_free(conv);
    free(pcm32);
    free(pcm);
}

/* Whole-file conversions: success with progress, cancels, refusals. */
static void file_cycle(const char *dsf, const char *wav, const char *junk, const char *missing,
                       int it) {
    static const uint32_t rates[] = {88200, 176400, 352800};
    dsd2pcm_file_options o = DSD2PCM_FILE_OPTIONS_INIT;
    o.rate_hz = rates[it % 3];
    o.cutoff = (uint32_t)(it / 3) % 2; /* standard or low */
    o.gain_db = it % 2 ? 3.0 : -6.0;
    o.progress = on_progress;
    for (uint32_t threads = 0; threads <= 1; threads++) {
        o.threads = threads;
        struct progress p = {0, 1 << 30, 0, 0};
        o.user_data = &p;
        dsd2pcm_file_report rep = DSD2PCM_FILE_REPORT_INIT;
        CHECK(dsd2pcm_convert_file(dsf, wav, &o, &rep) == DSD2PCM_OK);
        CHECK(p.calls >= 2 && p.last_done == p.total && p.total == rep.out_samples);
        /* Cancelled before and after the first block. */
        for (int stop = 0; stop <= 1; stop++) {
            struct progress c = {0, stop, 0, 0};
            o.user_data = &c;
            CHECK(dsd2pcm_convert_file(dsf, wav, &o, NULL) == DSD2PCM_E_CANCELLED);
            CHECK(c.calls == stop + 1);
        }
    }
    /* pysacd's filter, then the refusals. */
    dsd2pcm_file_options py = DSD2PCM_FILE_OPTIONS_INIT;
    py.rate_hz = 88200;
    py.filter = DSD2PCM_FILTER_PYSACD;
    py.threads = (uint32_t)(it % 2);
    CHECK(dsd2pcm_convert_file(dsf, wav, &py, NULL) == DSD2PCM_OK);
    o.user_data = NULL;
    o.progress = NULL;
    CHECK(dsd2pcm_convert_file(missing, wav, &o, NULL) == DSD2PCM_E_IO);
    CHECK(dsd2pcm_convert_file(junk, wav, &o, NULL) == DSD2PCM_E_FORMAT);
    CHECK(dsd2pcm_convert_file(dsf, dsf, &o, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    CHECK(dsd2pcm_convert_file(NULL, wav, &o, NULL) == DSD2PCM_E_NULL);
    CHECK(dsd2pcm_convert_file(dsf, "", &o, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_file_options bad = o;
    bad.rate_hz = 44100;
    CHECK(dsd2pcm_convert_file(dsf, wav, &bad, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    bad = o;
    bad.threads = 2;
    CHECK(dsd2pcm_convert_file(dsf, wav, &bad, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    bad = o;
    bad.struct_size = 0;
    CHECK(dsd2pcm_convert_file(dsf, wav, &bad, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
}

/* A thread that sets its last error and ends: the message goes with the thread. */
static void *error_thread(void *arg) {
    (void)arg;
    dsd2pcm_frames_config cfg = DSD2PCM_FRAMES_CONFIG_INIT;
    cfg.rate_hz = 1;
    cfg.channels = 2;
    dsd2pcm_frames *conv = NULL;
    int rc = dsd2pcm_frames_new(&cfg, &conv);
    return (void *)(size_t)(rc == DSD2PCM_E_INVALID_ARGUMENT && conv == NULL &&
                            strlen(dsd2pcm_last_error()) > 0);
}

int main(int argc, char **argv) {
    if (argc > 3) {
        fprintf(stderr, "usage: leakloop [ITERATIONS [DIR]]\n");
        return 2;
    }
    const int iterations = argc >= 2 ? atoi(argv[1]) : 20;
    if (iterations < 2) {
        fprintf(stderr, "leakloop: ITERATIONS must be at least 2\n");
        return 2;
    }
    const char *dir = argc == 3 ? argv[2] : getenv("TMPDIR");
    if (!dir || !*dir) dir = "/tmp";
    char dsf[4096], wav[4096], junk[4096], missing[4096];
    const long pid = (long)getpid();
    snprintf(dsf, sizeof dsf, "%s/dsd2pcm-leakloop-%ld.dsf", dir, pid);
    snprintf(wav, sizeof wav, "%s/dsd2pcm-leakloop-%ld.wav", dir, pid);
    snprintf(junk, sizeof junk, "%s/dsd2pcm-leakloop-%ld.bin", dir, pid);
    snprintf(missing, sizeof missing, "%s/dsd2pcm-leakloop-%ld.none", dir, pid);
    CHECK(write_dsf(dsf, 40, 0x9E3779B97F4A7C15u) == 0);
    FILE *jf = fopen(junk, "wb");
    CHECK(jf != NULL);
    if (jf) {
        fputs("RIFX not an audio file at all, but long enough to read a header from", jf);
        fclose(jf);
    }

    unsigned char *area = malloc((size_t)FRAMES * MAX_FRAME);
    uint64_t s = 0x2545F4914F6CDD1Du;
    for (size_t i = 0; i < (size_t)FRAMES * MAX_FRAME; i++) {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        area[i] = (i / 5000) % 4 == 3 ? 0xFF : (unsigned char)s;
    }

    static const uint32_t rates[] = {88200, 176400, 352800};
    static const double gains[] = {0.0, 6.0, -20.0};
    long first_half_max = 0, second_half_max = 0;
    for (int it = 0; it < iterations; it++) {
        for (int r = 0; r < 3; r++)
            for (uint32_t cutoff = 0; cutoff <= 2; cutoff++)
                for (uint32_t filter = 0; filter <= DSD2PCM_FILTER_FAST; filter++) {
                    uint32_t ch = 1 + (uint32_t)(it + r + (int)cutoff) % DSD2PCM_MAX_CHANNELS;
                    frames_cycle(rates[r], cutoff, filter, ch, gains[(it + r) % 3], area);
                }
        dsd2pcm_frames_free(NULL);
        file_cycle(dsf, wav, junk, missing, it);
        pthread_t t;
        void *ok = NULL;
        CHECK(pthread_create(&t, NULL, error_thread, NULL) == 0);
        CHECK(pthread_join(t, &ok) == 0 && ok != NULL);
        long rss = rss_kib();
        if (it >= iterations / 5 && it < iterations / 2 && rss > first_half_max)
            first_half_max = rss;
        if (it >= iterations / 2 && rss > second_half_max) second_half_max = rss;
    }
    printf("leakloop: %d iterations; RSS %ld KiB (20-50 %% max), %ld KiB (second-half max)\n",
           iterations, first_half_max, second_half_max);
#ifndef UNDER_ASAN
    if (first_half_max > 0) {
        long slack = first_half_max / 20 > 16384 ? first_half_max / 20 : 16384;
        CHECK(second_half_max <= first_half_max + slack);
    }
#endif

    free(area);
    remove(dsf);
    remove(wav);
    remove(junk);
    if (failures) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("leakloop: all checks passed\n");
    return 0;
}
