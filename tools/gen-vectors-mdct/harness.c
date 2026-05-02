#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mdct.h"
#include "vorbis/codec.h"

#define N 2048
#define N2 (N / 2)

static void write_floats(const char *path, const float *data, int count) {
    FILE *f = fopen(path, "wb");
    if (!f) {
        fprintf(stderr, "ERROR: cannot open %s\n", path);
        exit(1);
    }
    fwrite(data, sizeof(float), count, f);
    fclose(f);
}

static void run_case(mdct_lookup *lookup, const char *name, float in[N]) {
    float out[N2];
    char path_in[256];
    char path_out[256];

    mdct_forward(lookup, in, out);

    snprintf(path_in, sizeof(path_in), "input_%s.bin", name);
    snprintf(path_out, sizeof(path_out), "output_%s.bin", name);

    write_floats(path_in, in, N);
    write_floats(path_out, out, N2);
}

int main(void) {
    mdct_lookup lookup;
    mdct_init(&lookup, N);

    float in[N];
    int i;

    /* silence: all 0.0 */
    memset(in, 0, sizeof(in));
    run_case(&lookup, "silence", in);

    /* dc: all 0.5 */
    for (i = 0; i < N; i++) in[i] = 0.5f;
    run_case(&lookup, "dc", in);

    /* impulse: 1.0 at index 0, 0 elsewhere */
    memset(in, 0, sizeof(in));
    in[0] = 1.0f;
    run_case(&lookup, "impulse", in);

    /* ramp: i / 2048.0 for i in 0..2048 */
    for (i = 0; i < N; i++) in[i] = (float)i / (float)N;
    run_case(&lookup, "ramp", in);

    /* sine_440hz_44100: 0.5 * sin(2pi * 440 * i / 44100) */
    for (i = 0; i < N; i++)
        in[i] = 0.5f * (float)sin(2.0 * M_PI * 440.0 * i / 44100.0);
    run_case(&lookup, "sine_440hz_44100", in);

    /* negative_impulse: -1.0 at index 1024 */
    memset(in, 0, sizeof(in));
    in[1024] = -1.0f;
    run_case(&lookup, "negative_impulse", in);

    mdct_clear(&lookup);

    printf("OK: wrote 6 vector pairs\n");
    return 0;
}
