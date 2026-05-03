/*
 * dump_drft_tables — runs libvorbis's drft_init for n=256 and n=2048,
 * dumps the resulting trigcache + splitcache as raw bytes. Output is
 * read by the Python script that emits src/tables/drft.rs.
 *
 * Build: clang -O0 -ffp-contract=off -std=c99
 *        -I~/Documents/src/libvorbis/include -I~/Documents/src/libvorbis/lib
 *        ~/Documents/src/libvorbis/lib/smallft.c -lm
 *        -o dump_drft_tables
 *
 * Run: ./dump_drft_tables
 *      writes /tmp/c_drft_2048_trig.bin (24576 bytes), /tmp/c_drft_2048_split.bin (128),
 *             /tmp/c_drft_256_trig.bin (3072), /tmp/c_drft_256_split.bin (128).
 */
#include <stdio.h>
#include <stdlib.h>
#include "smallft.h"

static void dump(int n, const char *trig_path, const char *split_path) {
    drft_lookup l;
    drft_init(&l, n);
    /* trigcache is 3*n floats */
    FILE *f = fopen(trig_path, "wb");
    fwrite(l.trigcache, sizeof(float), 3 * n, f);
    fclose(f);
    /* splitcache is 32 ints */
    FILE *g = fopen(split_path, "wb");
    fwrite(l.splitcache, sizeof(int), 32, g);
    fclose(g);
    drft_clear(&l);
    fprintf(stderr, "n=%d: wrote %s + %s\n", n, trig_path, split_path);
}

int main(void) {
    dump(2048, "/tmp/c_drft_2048_trig.bin", "/tmp/c_drft_2048_split.bin");
    dump(256, "/tmp/c_drft_256_trig.bin", "/tmp/c_drft_256_split.bin");
    return 0;
}
