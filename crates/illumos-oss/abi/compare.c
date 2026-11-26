/*
 * Prints the OSS request numbers and structure sizes from the system header.
 * Build and run on illumos and compare with the table asserted in
 * src/illumos_oss/abi.rs:
 *
 *     gcc -o /tmp/oss-abi abi/compare.c && /tmp/oss-abi
 */
#include <sys/types.h>
#include <sys/soundcard.h>
#include <stdio.h>

#define	REQ(x)	printf("%-24s 0x%08x\n", #x, (unsigned int)(x))
#define	SZ(t)	printf("%-24s %zu\n", "sizeof " #t, sizeof (t))
#define	VAL(x)	printf("%-24s 0x%08x\n", #x, (unsigned int)(x))

int
main(void)
{
	SZ(audio_buf_info);
	SZ(audio_errinfo);
	SZ(oss_sysinfo);
	SZ(oss_audioinfo);
	REQ(SNDCTL_DSP_HALT);
	REQ(SNDCTL_DSP_SPEED);
	REQ(SNDCTL_DSP_SETFMT);
	REQ(SNDCTL_DSP_CHANNELS);
	REQ(SNDCTL_DSP_SETFRAGMENT);
	REQ(SNDCTL_DSP_GETOSPACE);
	REQ(SNDCTL_DSP_GETISPACE);
	REQ(SNDCTL_DSP_GETODELAY);
	REQ(SNDCTL_DSP_GETERROR);
	REQ(SNDCTL_SYSINFO);
	REQ(SNDCTL_AUDIOINFO);
	VAL(AFMT_S16_LE);
	VAL(AFMT_S32_LE);
	VAL(AFMT_S24_LE);
	return (0);
}
