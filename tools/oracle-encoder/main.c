/*
 * oracle-encoder — own-built libvorbis Q5 encoder, pinned compile flags.
 *
 * Reads i16 LE PCM (interleaved) from stdin, writes Ogg Vorbis to stdout.
 * Args: <rate> <channels>     e.g. ./oracle-encoder 44100 1
 *
 * Static-linked against ~/Documents/src/libvorbis/lib/*.c with:
 *     -O0 -ffp-contract=off -std=c99
 * so its byte output is fully deterministic and matches lewtoff's no-FMA
 * arithmetic. Use this as the parity oracle in tests/parity.rs instead of
 * system ffmpeg.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <unistd.h>

#include "vorbis/codec.h"
#include "vorbis/vorbisenc.h"

#define READ_CHUNK 1024

static void die(const char *msg) {
    fprintf(stderr, "oracle-encoder: %s\n", msg);
    exit(1);
}

int main(int argc, char **argv) {
    if (argc != 3) die("usage: oracle-encoder <rate> <channels>");
    long rate = atol(argv[1]);
    int channels = atoi(argv[2]);
    if (rate <= 0 || channels < 1 || channels > 2) die("invalid rate/channels");

    /* Read all i16 PCM from stdin. */
    size_t cap = 1 << 16;
    int16_t *samples = malloc(cap * sizeof(int16_t));
    size_t total = 0;
    for (;;) {
        if (total + READ_CHUNK > cap) {
            cap *= 2;
            samples = realloc(samples, cap * sizeof(int16_t));
        }
        ssize_t got = read(0, samples + total, READ_CHUNK * sizeof(int16_t));
        if (got < 0) die("read failed");
        if (got == 0) break;
        total += got / sizeof(int16_t);
    }
    long frames = (long)(total / channels);

    /* libvorbis init at quality 5. */
    vorbis_info vi;
    vorbis_info_init(&vi);
    /* q5 == base_quality 0.5 (libvorbis maps -0.1..1.0 to q-1..q10). */
    if (vorbis_encode_init_vbr(&vi, channels, rate, 0.5f) != 0)
        die("vorbis_encode_init_vbr failed");

    vorbis_comment vc;
    vorbis_comment_init(&vc);

    vorbis_dsp_state vd;
    vorbis_block vb;
    vorbis_analysis_init(&vd, &vi);
    vorbis_block_init(&vd, &vb);

    /* Write headers. */
    ogg_stream_state os;
    /* Stable serial for repeatability. The Rust parity test extracts
     * bytes 14..18 of our output and uses that serial in encode_with_serial. */
    ogg_stream_init(&os, 0xC0FFEE42);

    ogg_packet hdr, hdr_comm, hdr_code;
    vorbis_analysis_headerout(&vd, &vc, &hdr, &hdr_comm, &hdr_code);
    ogg_stream_packetin(&os, &hdr);
    ogg_stream_packetin(&os, &hdr_comm);
    ogg_stream_packetin(&os, &hdr_code);

    ogg_page og;
    while (ogg_stream_flush(&os, &og)) {
        fwrite(og.header, 1, og.header_len, stdout);
        fwrite(og.body, 1, og.body_len, stdout);
    }

    /* Feed PCM. */
    long fed = 0;
    int eos = 0;
    while (!eos) {
        if (fed < frames) {
            long this_chunk = READ_CHUNK;
            if (fed + this_chunk > frames) this_chunk = frames - fed;
            float **buffer = vorbis_analysis_buffer(&vd, this_chunk);
            for (long i = 0; i < this_chunk; i++) {
                for (int c = 0; c < channels; c++) {
                    buffer[c][i] = samples[(fed + i) * channels + c] / 32768.0f;
                }
            }
            vorbis_analysis_wrote(&vd, this_chunk);
            fed += this_chunk;
        } else {
            vorbis_analysis_wrote(&vd, 0);
        }

        ogg_packet op;
        while (vorbis_analysis_blockout(&vd, &vb) == 1) {
            vorbis_analysis(&vb, NULL);
            vorbis_bitrate_addblock(&vb);
            while (vorbis_bitrate_flushpacket(&vd, &op)) {
                ogg_stream_packetin(&os, &op);
                while (ogg_stream_pageout(&os, &og)) {
                    fwrite(og.header, 1, og.header_len, stdout);
                    fwrite(og.body, 1, og.body_len, stdout);
                    if (ogg_page_eos(&og)) eos = 1;
                }
            }
        }
    }
    while (ogg_stream_flush(&os, &og)) {
        fwrite(og.header, 1, og.header_len, stdout);
        fwrite(og.body, 1, og.body_len, stdout);
    }

    ogg_stream_clear(&os);
    vorbis_block_clear(&vb);
    vorbis_dsp_clear(&vd);
    vorbis_comment_clear(&vc);
    vorbis_info_clear(&vi);
    free(samples);
    return 0;
}
