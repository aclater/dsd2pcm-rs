/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/*
 * Smoke test of the C interface, built against include/dsd2pcm.h and the static library
 * by check.sh (gcc, clang, and clang with AddressSanitizer).
 *
 * It makes a stereo area of pseudo-random frames, writes it as a DSF file, and checks:
 * whole-file conversion with progress and cancel; the frame-run converter on the whole area
 * (no neighbours) gives the WAV file's samples; contiguous and per-frame buffers and runs
 * with neighbours give the same bytes; the same with pysacd's filter, whose dither
 * depends on first_frame; error codes and messages for bad arguments.
 *
 * Usage: smoke DIR (a writable directory for the test files).
 */
#include "dsd2pcm.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CH 2
#define FRAMES 20
#define FRAME (CH * DSD2PCM_FRAME_BYTES_PER_CHANNEL)

static int failures;

#define CHECK(cond)                                                                        \
    do {                                                                                   \
        if (!(cond)) {                                                                     \
            fprintf(stderr, "%s:%d: check failed: %s (last error: %s)\n", __FILE__,        \
                    __LINE__, #cond, dsd2pcm_last_error());                                \
            failures++;                                                                    \
        }                                                                                  \
    } while (0)

static void put32(unsigned char *p, uint32_t v) {
    for (int i = 0; i < 4; i++) p[i] = (unsigned char)(v >> (8 * i));
}

static void put64(unsigned char *p, uint64_t v) {
    for (int i = 0; i < 8; i++) p[i] = (unsigned char)(v >> (8 * i));
}

static unsigned char reverse_bits(unsigned char b) {
    unsigned char r = 0;
    for (int i = 0; i < 8; i++) r |= (unsigned char)(((b >> i) & 1) << (7 - i));
    return r;
}

/* Writes the interleaved, MSB-first area as a DSF file (LSB-first, 4096-byte blocks). */
static int write_dsf(const char *path, const unsigned char *area) {
    const size_t per_ch = (size_t)FRAMES * DSD2PCM_FRAME_BYTES_PER_CHANNEL;
    const size_t block = 4096;
    const size_t blocks = (per_ch + block - 1) / block;
    const size_t data = blocks * block * CH;
    unsigned char hdr[28 + 52 + 12] = {0};
    memcpy(hdr, "DSD ", 4);
    put64(hdr + 4, 28);
    put64(hdr + 12, sizeof hdr + data);
    memcpy(hdr + 28, "fmt ", 4);
    put64(hdr + 32, 52);
    put32(hdr + 40, 1);          /* version */
    put32(hdr + 44, 0);          /* raw DSD */
    put32(hdr + 48, 2);          /* stereo */
    put32(hdr + 52, CH);
    put32(hdr + 56, 2822400);
    put32(hdr + 60, 1);          /* LSB first */
    put64(hdr + 64, (uint64_t)per_ch * 8);
    put32(hdr + 72, (uint32_t)block);
    memcpy(hdr + 80, "data", 4);
    put64(hdr + 84, 12 + data);
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    fwrite(hdr, 1, sizeof hdr, f);
    unsigned char *buf = calloc(block, 1);
    if (!buf) {
        fclose(f);
        return -1;
    }
    for (size_t b = 0; b < blocks; b++) {
        for (size_t c = 0; c < CH; c++) {
            memset(buf, 0, block);
            for (size_t i = 0; i < block && b * block + i < per_ch; i++)
                buf[i] = reverse_bits(area[(b * block + i) * CH + c]);
            fwrite(buf, 1, block, f);
        }
    }
    free(buf);
    return fclose(f);
}

/* The bytes of the WAV file's data chunk; *len is set. */
static unsigned char *read_wav_data(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *all = malloc((size_t)n);
    if (!all || fread(all, 1, (size_t)n, f) != (size_t)n) {
        free(all);
        fclose(f);
        return NULL;
    }
    fclose(f);
    for (long at = 12; at + 8 <= n;) {
        uint32_t size = (uint32_t)all[at + 4] | (uint32_t)all[at + 5] << 8 |
                        (uint32_t)all[at + 6] << 16 | (uint32_t)all[at + 7] << 24;
        if (memcmp(all + at, "data", 4) == 0 && at + 8 + (long)size <= n) {
            unsigned char *d = malloc(size);
            if (d) memcpy(d, all + at + 8, size);
            *len = size;
            free(all);
            return d;
        }
        at += 8 + (long)size + (size & 1);
    }
    free(all);
    return NULL;
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

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: smoke DIR\n");
        return 2;
    }
    char dsf[4096], wav[4096];
    snprintf(dsf, sizeof dsf, "%s/smoke.dsf", argv[1]);
    snprintf(wav, sizeof wav, "%s/smoke.wav", argv[1]);
    printf("dsd2pcm %s\n", dsd2pcm_version());

    /* Pseudo-random area with full-scale stretches (which clip at +6 dB). */
    unsigned char *area = malloc((size_t)FRAMES * FRAME);
    uint64_t s = 0x9E3779B97F4A7C15u;
    for (size_t i = 0; i < (size_t)FRAMES * FRAME; i++) {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        area[i] = ((i / CH / 3000) % 3 == 2) ? 0xFF : (unsigned char)s;
    }
    CHECK(write_dsf(dsf, area) == 0);

    /* Whole file, with progress. */
    struct progress prog = {0, 1 << 30, 0, 0};
    dsd2pcm_file_options opts = {.rate_hz = 176400, .gain_db = 6.0, .threads = 1,
                                 .progress = on_progress, .user_data = &prog};
    dsd2pcm_file_report rep;
    CHECK(dsd2pcm_convert_file(dsf, wav, &opts, &rep) == DSD2PCM_OK);
    CHECK(prog.calls >= 2 && prog.last_done == prog.total && prog.total == rep.out_samples);
    CHECK(rep.channels == CH && rep.out_rate == 176400 && rep.in_rate == 2822400);
    CHECK(rep.clipped_total > 0 && rep.clipped_total == rep.clipped[0] + rep.clipped[1]);
    size_t wav_len = 0;
    unsigned char *wav_data = read_wav_data(wav, &wav_len);
    CHECK(wav_data != NULL);

    /* Cancel from the callback. */
    struct progress stop = {0, 0, 0, 0};
    opts.user_data = &stop;
    CHECK(dsd2pcm_convert_file(dsf, wav, &opts, NULL) == DSD2PCM_E_CANCELLED);
    CHECK(stop.calls == 1);
    CHECK(strcmp(dsd2pcm_last_error(), "conversion cancelled") == 0);

    /* Frame runs, same settings. */
    dsd2pcm_frames_config cfg = {.rate_hz = 176400, .channels = CH, .gain_db = 6.0};
    dsd2pcm_frames *conv = NULL;
    CHECK(dsd2pcm_frames_new(&cfg, &conv) == DSD2PCM_OK && conv != NULL);
    CHECK(dsd2pcm_frames_frame_bytes(conv) == FRAME);
    CHECK(dsd2pcm_frames_samples_per_frame(conv) == 2352);
    size_t out_len = dsd2pcm_frames_output_bytes(conv, FRAMES, DSD2PCM_FORMAT_S24LE);
    CHECK(out_len == (size_t)FRAMES * 2352 * CH * 3);
    CHECK(out_len == wav_len);

    /* The whole area as one run equals the WAV file. */
    unsigned char *pcm = malloc(out_len);
    uint64_t frame_clipped[FRAMES];
    dsd2pcm_stats st;
    dsd2pcm_run run = {.frames = area, .frames_len = (size_t)FRAMES * FRAME};
    CHECK(dsd2pcm_frames_convert(conv, &run, DSD2PCM_FORMAT_S24LE, pcm, out_len,
                                 frame_clipped, FRAMES, &st) == DSD2PCM_OK);
    CHECK(wav_data && memcmp(pcm, wav_data, out_len) == 0);
    uint64_t sum = 0;
    for (int i = 0; i < FRAMES; i++) sum += frame_clipped[i];
    CHECK(sum == st.clipped_total && st.clipped_total == rep.clipped_total);

    /* Frames 3..14 with neighbours, one buffer per frame, as int32: the same samples. */
    const unsigned char *ptrs[12];
    for (int i = 0; i < 12; i++) ptrs[i] = area + (size_t)(3 + i) * FRAME;
    dsd2pcm_run part = {.before = area + 2 * FRAME, .before_len = FRAME,
                        .frame_ptrs = ptrs, .frame_count = 12,
                        .after = area + 15 * FRAME, .after_len = FRAME, .first_frame = 3};
    size_t n32 = dsd2pcm_frames_output_bytes(conv, 12, DSD2PCM_FORMAT_I32);
    int32_t *pcm32 = malloc(n32);
    CHECK(dsd2pcm_frames_convert(conv, &part, DSD2PCM_FORMAT_I32, pcm32, n32, NULL, 0,
                                 NULL) == DSD2PCM_OK);
    size_t first = (size_t)3 * 2352 * CH;
    int same = 1;
    for (size_t i = 0; i < n32 / 4; i++) {
        const unsigned char *b = pcm + 3 * (first + i);
        int32_t v = (int32_t)((uint32_t)b[0] << 8 | (uint32_t)b[1] << 16 | (uint32_t)b[2] << 24) >> 8;
        same &= v == pcm32[i];
    }
    CHECK(same);

    /* pysacd's filter: the file equals the whole area as one run, and frames 3..14 with
     * neighbours and first_frame = 3, in separate buffers, give the same bytes. */
    {
        dsd2pcm_file_options popts = {.rate_hz = 88200, .filter = DSD2PCM_FILTER_PYSACD,
                                      .gain_db = 3.0};
        dsd2pcm_file_report prep;
        CHECK(dsd2pcm_convert_file(dsf, wav, &popts, &prep) == DSD2PCM_OK);
        size_t plen = 0;
        unsigned char *pwav = read_wav_data(wav, &plen);
        dsd2pcm_frames_config pcfg = {.rate_hz = 88200, .channels = CH, .gain_db = 3.0,
                                      .filter = DSD2PCM_FILTER_PYSACD};
        dsd2pcm_frames *pconv = NULL;
        CHECK(dsd2pcm_frames_new(&pcfg, &pconv) == DSD2PCM_OK);
        size_t n = dsd2pcm_frames_output_bytes(pconv, FRAMES, DSD2PCM_FORMAT_S24LE);
        CHECK(pwav != NULL && n == plen);
        unsigned char *ppcm = malloc(n);
        dsd2pcm_stats pst;
        CHECK(dsd2pcm_frames_convert(pconv, &run, DSD2PCM_FORMAT_S24LE, ppcm, n, NULL, 0,
                                     &pst) == DSD2PCM_OK);
        CHECK(pwav && memcmp(ppcm, pwav, n) == 0);
        CHECK(pst.clipped_total == prep.clipped_total);
        size_t pn = dsd2pcm_frames_output_bytes(pconv, 12, DSD2PCM_FORMAT_S24LE);
        unsigned char *ppart = malloc(pn);
        CHECK(dsd2pcm_frames_convert(pconv, &part, DSD2PCM_FORMAT_S24LE, ppart, pn, NULL, 0,
                                     NULL) == DSD2PCM_OK);
        size_t off = (size_t)3 * dsd2pcm_frames_samples_per_frame(pconv) * CH * 3;
        CHECK(memcmp(ppart, ppcm + off, pn) == 0);
        part.first_frame = 4; /* the wrong position: other dither */
        CHECK(dsd2pcm_frames_convert(pconv, &part, DSD2PCM_FORMAT_S24LE, ppart, pn, NULL, 0,
                                     NULL) == DSD2PCM_OK);
        CHECK(memcmp(ppart, ppcm + off, pn) != 0);
        part.first_frame = 3;
        dsd2pcm_frames_free(pconv);
        free(ppart);
        free(ppcm);
        free(pwav);
    }

    /* Errors: NULLs, bad lengths, overlap; nothing written on an error. */
    CHECK(dsd2pcm_frames_convert(NULL, &run, 0, pcm, out_len, NULL, 0, NULL) == DSD2PCM_E_NULL);
    CHECK(strcmp(dsd2pcm_last_error(), "conv is NULL") == 0);
    CHECK(dsd2pcm_frames_convert(conv, NULL, 0, pcm, out_len, NULL, 0, NULL) == DSD2PCM_E_NULL);
    CHECK(dsd2pcm_frames_convert(conv, &run, 0, NULL, out_len, NULL, 0, NULL) == DSD2PCM_E_NULL);
    CHECK(dsd2pcm_frames_convert(conv, &run, 0, pcm, out_len - 1, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    CHECK(dsd2pcm_frames_convert(conv, &run, 9, pcm, out_len, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    CHECK(dsd2pcm_frames_convert(conv, &run, 0, pcm, out_len, frame_clipped, 3, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_run overlap = {.frames = pcm, .frames_len = FRAME};
    CHECK(dsd2pcm_frames_convert(conv, &overlap, 0, pcm + 1,
                                 dsd2pcm_frames_output_bytes(conv, 1, 0), NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_run short_run = {.frames = area, .frames_len = FRAME - 1};
    memset(pcm, 0xAB, out_len);
    CHECK(dsd2pcm_frames_convert(conv, &short_run, 0, pcm, out_len, NULL, 0, NULL) ==
          DSD2PCM_E_INVALID_ARGUMENT);
    CHECK(pcm[0] == 0xAB && pcm[out_len - 1] == 0xAB);
    CHECK(dsd2pcm_convert_file(NULL, wav, &opts, NULL) == DSD2PCM_E_NULL);
    CHECK(strcmp(dsd2pcm_last_error(), "input_path is NULL") == 0);
    dsd2pcm_file_options bad = {.rate_hz = 44100};
    CHECK(dsd2pcm_convert_file(dsf, wav, &bad, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    bad.rate_hz = 352800;
    bad.filter = DSD2PCM_FILTER_PYSACD; /* pysacd: 88.2 and 176.4 kHz only */
    CHECK(dsd2pcm_convert_file(dsf, wav, &bad, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    dsd2pcm_frames *none = conv;
    cfg.channels = 7;
    CHECK(dsd2pcm_frames_new(&cfg, &none) == DSD2PCM_E_INVALID_ARGUMENT && none == NULL);
    CHECK(strcmp(dsd2pcm_status_str(DSD2PCM_E_BUSY), "converter busy") == 0);

    dsd2pcm_frames_free(conv);
    dsd2pcm_frames_free(NULL);
    free(pcm32);
    free(pcm);
    free(wav_data);
    free(area);
    /* smoke.dsf stays for check.sh's run of convert.c. */
    remove(wav);
    if (failures) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("smoke: all checks passed\n");
    return 0;
}
