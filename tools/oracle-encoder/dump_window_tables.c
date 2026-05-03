/*
 * dump_window_tables — dumps libvorbis's vwin{256,2048} arrays as raw f32.
 * Output is read by gen-tables to embed exact source-literal precision into
 * src/tables/window.rs.
 *
 * Build: clang -O0 -ffp-contract=off -std=c99
 *        -I~/Documents/src/libvorbis/include -I~/Documents/src/libvorbis/lib
 *        ~/Documents/src/libvorbis/lib/window.c -lm
 *        -o dump_window_tables
 *
 * Run: ./dump_window_tables
 *      writes /tmp/c_vwin256.bin (1024 bytes), /tmp/c_vwin2048.bin (8192).
 */
#include <stdio.h>

extern const float *_vorbis_window_get(int n);

int main(void) {
    /* vwin[] is indexed 0..7 for sizes 64,128,256,512,1024,2048,4096,8192.
     * Each table holds N/2 floats (the half-window — the full window is
     * reconstructed by mirror symmetry). */
    const float *w256 = _vorbis_window_get(2);    /* 128 floats */
    const float *w2048 = _vorbis_window_get(5);   /* 1024 floats */

    FILE *f256 = fopen("/tmp/c_vwin256.bin", "wb");
    fwrite(w256, sizeof(float), 128, f256);
    fclose(f256);

    FILE *f2048 = fopen("/tmp/c_vwin2048.bin", "wb");
    fwrite(w2048, sizeof(float), 1024, f2048);
    fclose(f2048);

    fprintf(stderr, "wrote /tmp/c_vwin256.bin (128 floats) and "
                    "/tmp/c_vwin2048.bin (1024 floats)\n");
    return 0;
}
