/********************************************************************
 *                                                                  *
 * THIS FILE IS PART OF THE OggVorbis SOFTWARE CODEC SOURCE CODE.   *
 * USE, DISTRIBUTION AND REPRODUCTION OF THIS LIBRARY SOURCE IS     *
 * GOVERNED BY A BSD-STYLE SOURCE LICENSE INCLUDED WITH THIS SOURCE *
 * IN 'COPYING'. PLEASE READ THESE TERMS BEFORE DISTRIBUTING.       *
 *                                                                  *
 * THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2009             *
 * by the Xiph.Org Foundation https://xiph.org/                     *
 *                                                                  *
 ********************************************************************

  function: LPC low level routines

 ********************************************************************/

/* Some of these routines (autocorrelator, LPC coefficient estimator)
   are derived from code written by Jutta Degener and Carsten Bormann;
   thus we include their copyright below.  The entirety of this file
   is freely redistributable on the condition that both of these
   copyright notices are preserved without modification.  */

/* Preserved Copyright: *********************************************/

/* Copyright 1992, 1993, 1994 by Jutta Degener and Carsten Bormann,
Technische Universita"t Berlin

Any use of this software is permitted provided that this notice is not
removed and that neither the authors nor the Technische Universita"t
Berlin are deemed to have made any representations as to the
suitability of this software for any purpose nor are held responsible
for any defects of this software. THERE IS ABSOLUTELY NO WARRANTY FOR
THIS SOFTWARE.

As a matter of courtesy, the authors request to be informed about uses
this software has found, about bugs in this software, and about any
improvements that may be of general interest.

Berlin, 28.11.1994
Jutta Degener
Carsten Bormann

*********************************************************************/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "os.h"
#include "smallft.h"
#include "lpc.h"
#include "scales.h"
#include "misc.h"

/* Autocorrelation LPC coeff generation algorithm invented by
   N. Levinson in 1947, modified by J. Durbin in 1959. */

/* Input : n elements of time doamin data
   Output: m lpc coefficients, excitation energy */

float vorbis_lpc_from_data(float *data,float *lpci,int n,int m){
  double *aut=alloca(sizeof(*aut)*(m+1));
  double *lpc=alloca(sizeof(*lpc)*(m));
  double error;
  double epsilon;
  int i,j;

  /* autocorrelation, p+1 lag coefficients */
  j=m+1;
  while(j--){
    double d=0; /* double needed for accumulator depth */
    for(i=j;i<n;i++)d+=(double)data[i]*data[i-j];
    aut[j]=d;
  }
  /* One-shot dump of aut[] on first call. */
  {
    static int fired = 0;
    if (!fired) {
      fired = 1;
      FILE *f = fopen("/tmp/lewtoff-debug/c_lpc_aut.bin", "wb");
      if (f) { fwrite(aut, sizeof(double), m+1, f); fclose(f); }
      fprintf(stderr, "C_LPC: aut dumped (m=%d, n=%d)\n", m, n);
    }
  }

  /* Generate lpc coefficients from autocorr values */

  /* set our noise floor to about -100dB */
  error=aut[0] * (1. + 1e-10);
  epsilon=1e-9*aut[0]+1e-10;

  for(i=0;i<m;i++){
    double r= -aut[i+1];

    if(error<epsilon){
      memset(lpc+i,0,(m-i)*sizeof(*lpc));
      goto done;
    }

    /* Sum up this iteration's reflection coefficient; note that in
       Vorbis we don't save it.  If anyone wants to recycle this code
       and needs reflection coefficients, save the results of 'r' from
       each iteration. */

    for(j=0;j<i;j++)r-=lpc[j]*aut[i-j];
    r/=error;

    /* Update LPC coefficients and total error */

    lpc[i]=r;
    for(j=0;j<i/2;j++){
      double tmp=lpc[j];

      lpc[j]+=r*lpc[i-1-j];
      lpc[i-1-j]+=r*tmp;
    }
    if(i&1)lpc[j]+=lpc[j]*r;

    error*=1.-r*r;

  }

 done:

  /* slightly damp the filter */
  {
    double g = .99;
    double damp = g;
    for(j=0;j<m;j++){
      lpc[j]*=damp;
      damp*=g;
    }
  }

  for(j=0;j<m;j++)lpci[j]=(float)lpc[j];

  /* One-shot dump of lpc[] coefficients on first call. */
  {
    static int fired = 0;
    if (!fired) {
      fired = 1;
      FILE *f = fopen("/tmp/lewtoff-debug/c_lpc_coeffs.bin", "wb");
      if (f) { fwrite(lpc, sizeof(double), m, f); fclose(f); }
      FILE *f2 = fopen("/tmp/lewtoff-debug/c_lpc_coeffs_f32.bin", "wb");
      if (f2) { fwrite(lpci, sizeof(float), m, f2); fclose(f2); }
    }
  }

  /* we need the error value to know how big an impulse to hit the
     filter with later */

  return error;
}

void vorbis_lpc_predict(float *coeff,float *prime,int m,
                     float *data,long n){

  /* in: coeff[0...m-1] LPC coefficients
         prime[0...m-1] initial values (allocated size of n+m-1)
    out: data[0...n-1] data samples */

  long i,j,o,p;
  float y;
  float *work=alloca(sizeof(*work)*(m+n));

  if(!prime)
    for(i=0;i<m;i++)
      work[i]=0.f;
  else
    for(i=0;i<m;i++)
      work[i]=prime[i];

  /* One-shot dump of prime+coeff inputs to lpc_predict on first call. */
  {
    static int fired = 0;
    if (!fired) {
      fired = 1;
      FILE *f = fopen("/tmp/lewtoff-debug/c_lpc_predict_prime.bin", "wb");
      if (f) { fwrite(prime, sizeof(float), m, f); fclose(f); }
      FILE *f2 = fopen("/tmp/lewtoff-debug/c_lpc_predict_coeff.bin", "wb");
      if (f2) { fwrite(coeff, sizeof(float), m, f2); fclose(f2); }
    }
  }

  for(i=0;i<n;i++){
    y=0;
    o=i;
    p=m;
    for(j=0;j<m;j++)
      y-=work[o++]*coeff[--p];

    data[i]=work[o]=y;
  }

  /* One-shot dump of predict output on first call. */
  {
    static int dumped_out = 0;
    if (!dumped_out) {
      dumped_out = 1;
      FILE *f = fopen("/tmp/lewtoff-debug/c_lpc_predict_out.bin", "wb");
      if (f) { fwrite(data, sizeof(float), n, f); fclose(f); }
    }
  }
}
