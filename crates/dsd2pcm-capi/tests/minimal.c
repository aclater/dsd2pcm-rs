/* Converts one stereo frame of DSD silence to 24-bit PCM at 176.4 kHz. */
#include <stdio.h>
#include <string.h>

#include "dsd2pcm.h"

int main(void) {
    static unsigned char frame[2 * DSD2PCM_FRAME_BYTES_PER_CHANNEL];
    static unsigned char pcm[2352 * 2 * 3];
    memset(frame, 0x69, sizeof frame); /* the DSD idle pattern */

    dsd2pcm_frames_config cfg = DSD2PCM_FRAMES_CONFIG_INIT;
    cfg.rate_hz = 176400;
    cfg.channels = 2;
    dsd2pcm_frames *conv = NULL;
    if (dsd2pcm_frames_new(&cfg, &conv) != DSD2PCM_OK) {
        fprintf(stderr, "dsd2pcm: %s\n", dsd2pcm_last_error());
        return 1;
    }
    dsd2pcm_run run = DSD2PCM_RUN_INIT;
    run.frames = frame;
    run.frames_len = sizeof frame;
    int rc = dsd2pcm_frames_convert(conv, &run, DSD2PCM_FORMAT_S24LE, pcm, sizeof pcm,
                                    NULL, 0, NULL);
    if (rc != DSD2PCM_OK) {
        fprintf(stderr, "dsd2pcm: %s\n", dsd2pcm_last_error());
    } else {
        printf("dsd2pcm %s: %zu samples per channel, first byte %d\n", dsd2pcm_version(),
               dsd2pcm_frames_samples_per_frame(conv), pcm[0]);
    }
    dsd2pcm_frames_free(conv);
    return rc;
}
