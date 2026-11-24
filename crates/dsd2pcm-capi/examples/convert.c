/* Converts a DSF or DFF file to a 24-bit WAV file at 176.4 kHz, printing progress.
 * Usage: convert IN.dsf OUT.wav */
#include <stdio.h>

#include "dsd2pcm.h"

static int progress(uint64_t done, uint64_t total, void *user_data) {
    (void)user_data;
    fprintf(stderr, "\r%3d%%", total ? (int)(100 * done / total) : 100);
    return 0; /* any other value cancels */
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: convert IN.dsf OUT.wav\n");
        return 2;
    }
    dsd2pcm_file_options opts = DSD2PCM_FILE_OPTIONS_INIT;
    opts.rate_hz = 176400;
    opts.progress = progress;
    dsd2pcm_file_report report = DSD2PCM_FILE_REPORT_INIT;
    int rc = dsd2pcm_convert_file(argv[1], argv[2], &opts, &report);
    fprintf(stderr, "\n");
    if (rc != DSD2PCM_OK) {
        fprintf(stderr, "dsd2pcm: %s\n", dsd2pcm_last_error());
        return 1;
    }
    printf("%.2f s, %llu samples per channel, %llu clipped\n", report.seconds,
           (unsigned long long)report.out_samples, (unsigned long long)report.clipped_total);
    return 0;
}
