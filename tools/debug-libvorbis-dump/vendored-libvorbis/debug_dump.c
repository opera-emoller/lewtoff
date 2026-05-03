#include "debug_dump.h"

int dump_now = 0;
int dump_done = 0;

float g_windowed_pcm[DUMP_MAX_HALF * 2];
int   g_windowed_n = 0;
float g_drft_output[DUMP_MAX_HALF * 2];
float g_logfft[DUMP_MAX_HALF + 1];
float g_logmdct[DUMP_MAX_HALF + 1];
float g_logmask[DUMP_MAX_HALF + 1];
float g_noise[DUMP_MAX_HALF + 1];
float g_tone[DUMP_MAX_HALF + 1];
int   g_floor_posts[DUMP_MAX_POSTS];
int   g_floor_post_count = 0;
BitRecord g_floor_bits[DUMP_MAX_BITS];
int   g_floor_bits_count = 0;
