/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/*
 * struct_size versioning, as a caller built against another header passes it. Built by
 * check.sh (gcc, clang, and clang with AddressSanitizer and UndefinedBehaviorSanitizer).
 * Every struct handed to the library is a heap block of exactly struct_size bytes, so a
 * read past it shows under AddressSanitizer.
 *
 * - Old size: dsd2pcm_file_options as the 0.1.0 header had it (it ends before
 *   no_clobber). Accepted; whatever lies after it is not read, so no_clobber is 0 and an
 *   existing output is replaced.
 * - Current size: no_clobber 1 refuses an existing output with DSD2PCM_E_EXISTS.
 * - Larger (a newer header) with the extra bytes 0: accepted, for every input struct;
 *   an output struct's extra bytes are left as they are.
 * - Larger with a non-zero extra byte: refused with DSD2PCM_E_INVALID_ARGUMENT before
 *   anything is done.
 * - Below the 0.1.0 size or above 4096 bytes: refused.
 *
 * Usage: struct_size [DIR] (a writable directory; default $TMPDIR or /tmp).
 */
#include "dsd2pcm.h"

#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

/* A mono DSD64 DSF file of one 4096-byte block of the idle pattern. */
static int write_dsf(const char *path) {
    unsigned char hdr[28 + 52 + 12] = {0};
    unsigned char block[4096];
    memset(block, 0x96, sizeof block); /* 0x69 bit-reversed: DSF is LSB first */
    memcpy(hdr, "DSD ", 4);
    put64(hdr + 4, 28);
    put64(hdr + 12, sizeof hdr + sizeof block);
    memcpy(hdr + 28, "fmt ", 4);
    put64(hdr + 32, 52);
    put32(hdr + 40, 1);       /* version */
    put32(hdr + 44, 0);       /* raw DSD */
    put32(hdr + 48, 1);       /* mono */
    put32(hdr + 52, 1);       /* channels */
    put32(hdr + 56, 2822400); /* DSD64 */
    put32(hdr + 60, 1);       /* LSB first */
    put64(hdr + 64, (uint64_t)sizeof block * 8);
    put32(hdr + 72, (uint32_t)sizeof block);
    memcpy(hdr + 80, "data", 4);
    put64(hdr + 84, 12 + sizeof block);
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    int ok = fwrite(hdr, 1, sizeof hdr, f) == sizeof hdr &&
             fwrite(block, 1, sizeof block, f) == sizeof block;
    return fclose(f) == 0 && ok ? 0 : -1;
}

static int write_text(const char *path, const char *text) {
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    int ok = fputs(text, f) >= 0;
    return fclose(f) == 0 && ok ? 0 : -1;
}

/* 1 if the file starts with `text`. */
static int starts_with(const char *path, const char *text) {
    char buf[64] = {0};
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    size_t n = fread(buf, 1, sizeof buf - 1, f);
    fclose(f);
    return n >= strlen(text) && memcmp(buf, text, strlen(text)) == 0;
}

/* A heap copy of `size` bytes: the first min(size, len) bytes of `src`, then zeros, with
 * struct_size (the first field of every struct) set to `size`. */
static void *sized(const void *src, size_t len, size_t size) {
    unsigned char *p = calloc(1, size);
    if (!p) {
        perror("calloc");
        exit(1);
    }
    memcpy(p, src, len < size ? len : size);
    uint32_t s = (uint32_t)size;
    memcpy(p, &s, sizeof s);
    return p;
}

int main(int argc, char **argv) {
    if (argc > 2) {
        fprintf(stderr, "usage: struct_size [DIR]\n");
        return 2;
    }
    const char *dir = argc == 2 ? argv[1] : getenv("TMPDIR");
    if (!dir || !*dir) dir = "/tmp";
    char dsf[4096], wav[4096];
    snprintf(dsf, sizeof dsf, "%s/dsd2pcm-struct-size.dsf", dir);
    snprintf(wav, sizeof wav, "%s/dsd2pcm-struct-size.wav", dir);
    CHECK(write_dsf(dsf) == 0);

    /* ---- dsd2pcm_file_options ---- */
    dsd2pcm_file_options opts = DSD2PCM_FILE_OPTIONS_INIT;
    opts.rate_hz = 88200;
    opts.threads = 1;
    opts.no_clobber = 1;
    const size_t v1 = offsetof(dsd2pcm_file_options, no_clobber); /* the 0.1.0 size */
    const size_t now = sizeof opts;
    const size_t big = now + 24;

    /* Current size, no_clobber 1: an existing output is refused and kept. */
    CHECK(write_text(wav, "precious") == 0);
    CHECK(dsd2pcm_convert_file(dsf, wav, &opts, NULL) == DSD2PCM_E_EXISTS);
    CHECK(starts_with(wav, "precious"));

    /* The 0.1.0 size: no_clobber is not read (it is past the caller's struct), so the
     * output is replaced, as 0.1.0 did. The block ends at the struct's end. */
    void *old = sized(&opts, v1, v1);
    CHECK(dsd2pcm_convert_file(dsf, wav, old, NULL) == DSD2PCM_OK);
    CHECK(starts_with(wav, "RIFF"));
    free(old);
    /* The same with a 1 right after the old struct, where no_clobber would be. */
    CHECK(write_text(wav, "precious") == 0);
    old = sized(&opts, now, now);
    *(uint32_t *)old = (uint32_t)v1;
    CHECK(((dsd2pcm_file_options *)old)->no_clobber == 1);
    CHECK(dsd2pcm_convert_file(dsf, wav, old, NULL) == DSD2PCM_OK);
    CHECK(starts_with(wav, "RIFF"));
    free(old);

    /* Larger, extra bytes 0: accepted, and no_clobber read. */
    CHECK(write_text(wav, "precious") == 0);
    void *newer = sized(&opts, now, big);
    CHECK(dsd2pcm_convert_file(dsf, wav, newer, NULL) == DSD2PCM_E_EXISTS);
    CHECK(starts_with(wav, "precious"));
    ((dsd2pcm_file_options *)newer)->no_clobber = 0;
    CHECK(dsd2pcm_convert_file(dsf, wav, newer, NULL) == DSD2PCM_OK);
    CHECK(starts_with(wav, "RIFF"));

    /* Larger with a non-zero extra byte (first and last): refused, nothing written. */
    for (size_t at = now; at < big; at += big - now - 1) {
        CHECK(write_text(wav, "precious") == 0);
        ((unsigned char *)newer)[at] = 1;
        CHECK(dsd2pcm_convert_file(dsf, wav, newer, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
        CHECK(strstr(dsd2pcm_last_error(), "is not 0") != NULL);
        CHECK(starts_with(wav, "precious"));
        ((unsigned char *)newer)[at] = 0;
    }
    free(newer);

    /* Too small, too large. */
    void *small = sized(&opts, v1 - 1, v1 - 1);
    CHECK(dsd2pcm_convert_file(dsf, wav, small, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    free(small);
    void *huge = sized(&opts, now, 4097);
    CHECK(dsd2pcm_convert_file(dsf, wav, huge, NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    free(huge);
    void *max = sized(&opts, now, 4096);
    ((dsd2pcm_file_options *)max)->no_clobber = 0;
    CHECK(dsd2pcm_convert_file(dsf, wav, max, NULL) == DSD2PCM_OK);
    free(max);

    /* An output struct of a newer header: our fields written, its own bytes kept. */
    dsd2pcm_file_report rep = DSD2PCM_FILE_REPORT_INIT;
    unsigned char *rbig = sized(&rep, sizeof rep, sizeof rep + 16);
    memset(rbig + sizeof rep, 0xAB, 16);
    opts.no_clobber = 0;
    CHECK(dsd2pcm_convert_file(dsf, wav, &opts, (dsd2pcm_file_report *)rbig) == DSD2PCM_OK);
    CHECK(((dsd2pcm_file_report *)rbig)->struct_size == sizeof rep + 16);
    CHECK(((dsd2pcm_file_report *)rbig)->channels == 1);
    for (size_t i = sizeof rep; i < sizeof rep + 16; i++) CHECK(rbig[i] == 0xAB);
    free(rbig);

    /* ---- dsd2pcm_frames_config, dsd2pcm_run, dsd2pcm_stats ---- */
    dsd2pcm_frames_config cfg = DSD2PCM_FRAMES_CONFIG_INIT;
    cfg.rate_hz = 176400;
    cfg.channels = 1;
    dsd2pcm_frames *conv = NULL;
    void *cbig = sized(&cfg, sizeof cfg, sizeof cfg + 8);
    CHECK(dsd2pcm_frames_new(cbig, &conv) == DSD2PCM_OK && conv != NULL);
    dsd2pcm_frames_free(conv);
    conv = NULL;
    ((unsigned char *)cbig)[sizeof cfg + 7] = 0x80;
    CHECK(dsd2pcm_frames_new(cbig, &conv) == DSD2PCM_E_INVALID_ARGUMENT && conv == NULL);
    free(cbig);
    void *cnow = sized(&cfg, sizeof cfg, sizeof cfg);
    CHECK(dsd2pcm_frames_new(cnow, &conv) == DSD2PCM_OK && conv != NULL);
    free(cnow);

    static unsigned char frame[DSD2PCM_FRAME_BYTES_PER_CHANNEL];
    static unsigned char pcm[2352 * 3];
    memset(frame, 0x69, sizeof frame);
    dsd2pcm_run run = DSD2PCM_RUN_INIT;
    run.frames = frame;
    run.frames_len = sizeof frame;
    dsd2pcm_stats st = DSD2PCM_STATS_INIT;
    unsigned char *sbig = sized(&st, sizeof st, sizeof st + 8);
    memset(sbig + sizeof st, 0xCD, 8);
    void *rnow = sized(&run, sizeof run, sizeof run);
    CHECK(dsd2pcm_frames_convert(conv, rnow, DSD2PCM_FORMAT_S24LE, pcm, sizeof pcm, NULL, 0,
                                 NULL) == DSD2PCM_OK);
    free(rnow);
    void *rbig2 = sized(&run, sizeof run, sizeof run + 16);
    CHECK(dsd2pcm_frames_convert(conv, rbig2, DSD2PCM_FORMAT_S24LE, pcm, sizeof pcm, NULL, 0,
                                 (dsd2pcm_stats *)sbig) == DSD2PCM_OK);
    CHECK(((dsd2pcm_stats *)sbig)->channels == 1);
    for (size_t i = sizeof st; i < sizeof st + 8; i++) CHECK(sbig[i] == 0xCD);
    ((unsigned char *)rbig2)[sizeof run] = 1;
    CHECK(dsd2pcm_frames_convert(conv, rbig2, DSD2PCM_FORMAT_S24LE, pcm, sizeof pcm, NULL, 0,
                                 NULL) == DSD2PCM_E_INVALID_ARGUMENT);
    free(rbig2);
    free(sbig);
    dsd2pcm_frames_free(conv);

    remove(dsf);
    remove(wav);
    if (failures) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("struct_size checks passed\n");
    return 0;
}
