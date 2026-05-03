/* corpus_harness.c — feed a raw stereo s16le 44100 stream from stdin
 * through libvorbis Q5 stereo with chunk=64. Stderr will carry the
 * C_AMP debug output from the vendored envelope.c.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "vendored-libvorbis/include/vorbis/codec.h"
#include "vendored-libvorbis/include/vorbis/vorbisenc.h"

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: %s out.ogg < raw.s16le.stereo.44100\n", argv[0]); return 1; }

    FILE *out = fopen(argv[1], "wb");
    if (!out) { perror("fopen"); return 1; }

    vorbis_info vi; vorbis_info_init(&vi);
    if (vorbis_encode_init_vbr(&vi, 2, 44100, 0.5f)) { fprintf(stderr, "init_vbr failed\n"); return 1; }
    vorbis_comment vc; vorbis_comment_init(&vc);
    vorbis_dsp_state vd; vorbis_block vb;
    vorbis_analysis_init(&vd, &vi); vorbis_block_init(&vd, &vb);

    ogg_stream_state os; ogg_stream_init(&os, 0xc0ffee42);
    ogg_packet h, hc, hcd; vorbis_analysis_headerout(&vd, &vc, &h, &hc, &hcd);
    ogg_stream_packetin(&os, &h);
    ogg_stream_packetin(&os, &hc);
    ogg_stream_packetin(&os, &hcd);
    ogg_page og;
    while (ogg_stream_flush(&os, &og)) { fwrite(og.header, 1, og.header_len, out); fwrite(og.body, 1, og.body_len, out); }

    const int CHUNK = 64;
    int16_t buf[CHUNK * 2];
    int eos = 0;

    while (!eos) {
        size_t got = fread(buf, sizeof(int16_t) * 2, CHUNK, stdin);
        if (got == 0) {
            vorbis_analysis_wrote(&vd, 0); eos = 1;
        } else {
            float **pcm = vorbis_analysis_buffer(&vd, (int)got);
            for (size_t i = 0; i < got; i++) {
                pcm[0][i] = buf[i*2]   / 32768.0f;
                pcm[1][i] = buf[i*2+1] / 32768.0f;
            }
            vorbis_analysis_wrote(&vd, (int)got);
        }
        while (vorbis_analysis_blockout(&vd, &vb) == 1) {
            vorbis_analysis(&vb, NULL); vorbis_bitrate_addblock(&vb);
            ogg_packet op;
            while (vorbis_bitrate_flushpacket(&vd, &op)) {
                ogg_stream_packetin(&os, &op);
                while (ogg_stream_pageout(&os, &og)) {
                    fwrite(og.header, 1, og.header_len, out);
                    fwrite(og.body, 1, og.body_len, out);
                }
            }
        }
    }
    while (ogg_stream_flush(&os, &og)) { fwrite(og.header, 1, og.header_len, out); fwrite(og.body, 1, og.body_len, out); }

    fclose(out);
    ogg_stream_clear(&os);
    vorbis_block_clear(&vb); vorbis_dsp_clear(&vd);
    vorbis_comment_clear(&vc); vorbis_info_clear(&vi);
    return 0;
}
