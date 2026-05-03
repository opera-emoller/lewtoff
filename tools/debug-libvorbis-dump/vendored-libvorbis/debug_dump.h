#pragma once

#define DUMP_MAX_HALF 512
#define DUMP_MAX_BITS 4096
#define DUMP_MAX_POSTS 65   /* VIF_POSIT(63) + 2 */

typedef struct { int value; int bits; } BitRecord;

extern int dump_now;
extern int dump_done;

extern float g_windowed_pcm[DUMP_MAX_HALF * 2];
extern int   g_windowed_n;
extern float g_drft_output[DUMP_MAX_HALF * 2];
extern float g_logfft[DUMP_MAX_HALF + 1];
extern float g_logmdct[DUMP_MAX_HALF + 1];
extern float g_logmask[DUMP_MAX_HALF + 1];
extern int   g_floor_posts[DUMP_MAX_POSTS];
extern int   g_floor_post_count;
extern BitRecord g_floor_bits[DUMP_MAX_BITS];
extern int        g_floor_bits_count;
