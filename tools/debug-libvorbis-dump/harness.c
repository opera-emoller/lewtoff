/*
 * harness.c — debug dump harness for the first short block of 440Hz sine.
 * Feeds 1 second of 440Hz mono 44100Hz sine through libvorbis (Q5) and
 * captures the first short block's intermediate state to /tmp/lewtoff-debug/.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdint.h>

#include "vendored-libvorbis/include/vorbis/codec.h"
#include "vendored-libvorbis/include/vorbis/vorbisenc.h"
#include "vendored-libvorbis/debug_dump.h"

#define OUTDIR "/tmp/lewtoff-debug"

static void write_f32(const char *name, const float *data, int n) {
    char path[256];
    snprintf(path, sizeof(path), OUTDIR "/%s", name);
    FILE *f = fopen(path, "wb");
    if (!f) { fprintf(stderr, "ERROR: cannot open %s\n", path); return; }
    fwrite(data, sizeof(float), n, f);
    fclose(f);
    fprintf(stderr, "wrote %s (%d floats)\n", path, n);
}

static void write_i32(const char *name, const int *data, int n) {
    char path[256];
    snprintf(path, sizeof(path), OUTDIR "/%s", name);
    FILE *f = fopen(path, "wb");
    if (!f) { fprintf(stderr, "ERROR: cannot open %s\n", path); return; }
    fwrite(data, sizeof(int), n, f);
    fclose(f);
    fprintf(stderr, "wrote %s (%d ints)\n", path, n);
}

static void write_txt(const char *name, const char *text) {
    char path[256];
    snprintf(path, sizeof(path), OUTDIR "/%s", name);
    FILE *f = fopen(path, "w");
    if (!f) { fprintf(stderr, "ERROR: cannot open %s\n", path); return; }
    fputs(text, f);
    fclose(f);
}

int main(void) {
    system("mkdir -p " OUTDIR);

    vorbis_info vi;
    vorbis_info_init(&vi);
    int ret = vorbis_encode_init_vbr(&vi, 1, 44100, 0.5f);
    if (ret) { fprintf(stderr, "vorbis_encode_init_vbr failed: %d\n", ret); return 1; }

    vorbis_comment vc;
    vorbis_comment_init(&vc);

    vorbis_dsp_state vd;
    vorbis_block vb;
    vorbis_analysis_init(&vd, &vi);
    vorbis_block_init(&vd, &vb);

    ogg_packet header, header_comm, header_code;
    vorbis_analysis_headerout(&vd, &vc, &header, &header_comm, &header_code);

    const int RATE = 44100;
    const int FRAMES = RATE;
    const int CHUNK = 64;

    int fed = 0;
    int packet_count = 0;
    int first_short_done = 0;

    int eos_signaled = 0;
    int eos_blocks_drained = 0;
    while (1) {
        if (fed < FRAMES) {
            int this_chunk = CHUNK;
            if (fed + this_chunk > FRAMES) this_chunk = FRAMES - fed;

            float **pcm = vorbis_analysis_buffer(&vd, this_chunk);
            for (int i = 0; i < this_chunk; i++) {
                /* Match Rust parity test EXACTLY:
                 *   let t = i as f32 / rate as f32;     // division FIRST
                 *   let v = f32::sin(2.0 * PI * freq * t);
                 *   let s = (v * 16384.0) as i16;
                 *   let pcm = s as f32 / 32768.0;
                 */
                float t = (float)(fed + i) / (float)RATE;
                float v = sinf(2.0f * (float)M_PI * 440.0f * t);
                int16_t s = (int16_t)(v * 16384.0f);
                pcm[0][i] = s / 32768.0f;
                if((fed+i)<10) fprintf(stderr,"HARNESS_S idx=%d s=%d\n",fed+i,s);
            }
            vorbis_analysis_wrote(&vd, this_chunk);
            fed += this_chunk;
        } else {
            if(!eos_signaled){
                vorbis_analysis_wrote(&vd, 0);
                eos_signaled = 1;
            }
        }

        while (vorbis_analysis_blockout(&vd, &vb) == 1) {
            if (!first_short_done && vb.W == 0 && vb.sequence >= 3) {
                dump_now = 1;
            } else {
                dump_now = 0;
            }

            vorbis_analysis(&vb, NULL);
            vorbis_bitrate_addblock(&vb);

            if (!first_short_done && dump_done) {
                first_short_done = 1;
                dump_now = 0;
                fprintf(stderr, "First short block encoded: seq=%lld W=%d n=%d posts=%d bits_entries=%d\n",
                        (long long)vb.sequence, vb.W, vb.pcmend, g_floor_post_count, g_floor_bits_count);

                write_f32("c_windowed.bin", g_windowed_pcm, g_windowed_n);
                write_f32("c_drft.bin",     g_drft_output,  g_windowed_n);

                int half = g_windowed_n / 2 + 1;
                write_f32("c_logfft.bin",   g_logfft,  half);
                write_f32("c_logmdct.bin",  g_logmdct, half);
                write_f32("c_mask.bin",     g_logmask, half);
                write_f32("c_noise.bin",    g_noise,   half);
                write_f32("c_tone.bin",     g_tone,    half);

                write_i32("c_floor_posts.bin", g_floor_posts, g_floor_post_count);

                {
                    char buf[64];
                    snprintf(buf, sizeof(buf), "%d\n", g_floor_post_count);
                    write_txt("c_floor_count.txt", buf);
                }

                {
                    char path[256];
                    snprintf(path, sizeof(path), OUTDIR "/c_floor_bits.txt");
                    FILE *f = fopen(path, "w");
                    if (f) {
                        int total_bits = 0;
                        for (int i = 0; i < g_floor_bits_count; i++) {
                            if(g_floor_bits[i].bits == -1){
                                fprintf(f, "# total_floor_bits=%d\n", g_floor_bits[i].value);
                                total_bits = g_floor_bits[i].value;
                            } else {
                                fprintf(f, "%d %d\n", g_floor_bits[i].value, g_floor_bits[i].bits);
                            }
                        }
                        fclose(f);
                        fprintf(stderr, "wrote %s (%d entries, total_floor_bits=%d)\n",
                                path, g_floor_bits_count, total_bits);
                    }
                }
            }

            ogg_packet op;
            while (vorbis_bitrate_flushpacket(&vd, &op)) {
                packet_count++;
                if(packet_count<=5){
                    fprintf(stderr,"HARNESS_PACKET seq=%lld bytes=%ld first=[",
                            (long long)op.packetno, op.bytes);
                    for(int z=0;z<op.bytes && z<24;z++)
                      fprintf(stderr,"%02x ",op.packet[z]);
                    fprintf(stderr,"]\n");
                }
            }
        }

        if (eos_signaled) {
            eos_blocks_drained++;
            if (eos_blocks_drained > 5) break;
        }
    }

    vorbis_block_clear(&vb);
    vorbis_dsp_clear(&vd);
    vorbis_comment_clear(&vc);
    vorbis_info_clear(&vi);

    printf("OK: dumped first short block (packet_count=%d)\n", packet_count);
    return 0;
}
