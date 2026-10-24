#include "vault_audio.h"
#include <stdio.h>
int main(int argc,char **argv){if(argc!=3)return 2;char error[512]={0};VaultAudioInfo info;if(vault_audio_inspect(argv[1],&info,error,sizeof(error))){fprintf(stderr,"%s\n",error);return 1;}if(vault_audio_prepare(argv[1],argv[2],NULL,NULL,error,sizeof(error))){fprintf(stderr,"%s\n",error);return 1;}printf("%u Hz %u-bit %u channels, %llu frames\n",info.sample_rate,info.bits,info.channels,(unsigned long long)info.frames);return 0;}
