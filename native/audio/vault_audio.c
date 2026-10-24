#include "vault_audio.h"
#include <FLAC/stream_encoder.h>
#include <FLAC/stream_decoder.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
static uint16_t u16(const unsigned char *p){return (uint16_t)(p[0]|p[1]<<8);}
static uint32_t u32(const unsigned char *p){return (uint32_t)p[0]|(uint32_t)p[1]<<8|(uint32_t)p[2]<<16|(uint32_t)p[3]<<24;}
static uint64_t u64(const unsigned char *p){return u32(p)|((uint64_t)u32(p+4)<<32);}
static int fail(char *out,int n,const char *message){if(out&&n>0)snprintf(out,(size_t)n,"%s",message);return -1;}
int vault_audio_inspect(const char *path,VaultAudioInfo *info,char *err,int n){
 memset(info,0,sizeof(*info));FILE *f=fopen(path,"rb");if(!f)return fail(err,n,"Cannot open source recording");
 unsigned char head[12];if(fread(head,1,12,f)!=12||memcmp(head+8,"WAVE",4)||(memcmp(head,"RIFF",4)&&memcmp(head,"RF64",4))){fclose(f);return fail(err,n,"Not a supported WAV file");}
 uint64_t rf64_size=0;int has_format=0,has_data=0;uint16_t encoding=0;
 while(!has_data){unsigned char h[8];if(fread(h,1,8,f)!=8)break;uint64_t length=u32(h+4);off_t start=ftello(f);
  if(!memcmp(h,"ds64",4)){unsigned char b[28];if(length<28||fread(b,1,28,f)!=28)break;rf64_size=u64(b+8);}
  if(!memcmp(h,"fmt ",4)){unsigned char b[64];size_t want=length<64?(size_t)length:64;if(want<16||fread(b,1,want,f)!=want)break;encoding=u16(b);info->channels=u16(b+2);info->sample_rate=u32(b+4);info->bits=u16(b+14);if(encoding==0xfffe&&want>=40)encoding=u16(b+24);has_format=1;}
  if(!memcmp(h,"data",4)){if(length==0xffffffff)length=rf64_size;info->data_offset=(uint64_t)start;info->data_bytes=length;has_data=1;break;}
  if(length>INT64_MAX-(uint64_t)start-1||fseeko(f,start+(off_t)length+(length&1),SEEK_SET))break;
 }
 struct stat s;int size_ok=!fstat(fileno(f),&s);fclose(f);
 if(!has_format||!has_data||!size_ok||(encoding!=1&&encoding!=3)||info->channels<1||info->channels>8||info->sample_rate<8000||info->sample_rate>384000||info->bits<8||info->bits>64||info->bits%8)return fail(err,n,"Unsupported or unfinished WAV header");
 uint64_t frame_bytes=(uint64_t)info->channels*info->bits/8;
 if(!info->data_bytes||info->data_offset>(uint64_t)s.st_size||info->data_bytes>(uint64_t)s.st_size-info->data_offset||info->data_bytes%frame_bytes)return fail(err,n,"Truncated or empty WAV audio");
 info->is_float=encoding==3;info->frames=info->data_bytes/frame_bytes;info->flac=!info->is_float&&info->bits<=24;return 0;
}
typedef struct {uint64_t frames;int failed;} Verify;
static FLAC__StreamDecoderWriteStatus decoded(const FLAC__StreamDecoder*d,const FLAC__Frame*frame,const FLAC__int32*const b[],void*ctx){(void)d;(void)b;((Verify*)ctx)->frames+=frame->header.blocksize;return FLAC__STREAM_DECODER_WRITE_STATUS_CONTINUE;}
static void decode_error(const FLAC__StreamDecoder*d,FLAC__StreamDecoderErrorStatus s,void*ctx){(void)d;(void)s;((Verify*)ctx)->failed=1;}
int vault_audio_prepare(const char *src,const char *dst,VaultProgress progress,void *context,char *err,int n){
 VaultAudioInfo info;if(vault_audio_inspect(src,&info,err,n))return -1;
 struct stat exists;if(!stat(dst,&exists))return fail(err,n,"Destination already exists");
 FILE *in=fopen(src,"rb");if(!in)return fail(err,n,"Source disconnected");
 int ok=1;unsigned char bytes[65536];uint64_t done=0;
 if(!info.flac){FILE*out=fopen(dst,"wb");if(!out){fclose(in);return fail(err,n,"Cannot create staged recording");}size_t read;while((read=fread(bytes,1,sizeof(bytes),in))>0){if(fwrite(bytes,1,read,out)!=read){ok=0;break;}if(progress&&progress((double)(done+=read)/(info.data_bytes+info.data_offset)*.6,context)){ok=0;break;}}if(ferror(in))ok=0;fflush(out);if(fsync(fileno(out)))ok=0;fclose(out);rewind(in);out=fopen(dst,"rb");if(!out)ok=0;while(ok&&(read=fread(bytes,1,sizeof(bytes),in))>0){unsigned char check[65536];if(fread(check,1,read,out)!=read||memcmp(bytes,check,read))ok=0;}if(out)fclose(out);
 }else{
  FLAC__StreamEncoder *encoder=FLAC__stream_encoder_new();if(!encoder){fclose(in);return fail(err,n,"Cannot allocate FLAC encoder");}
  FLAC__stream_encoder_set_channels(encoder,info.channels);FLAC__stream_encoder_set_bits_per_sample(encoder,info.bits);FLAC__stream_encoder_set_sample_rate(encoder,info.sample_rate);FLAC__stream_encoder_set_total_samples_estimate(encoder,info.frames);FLAC__stream_encoder_set_compression_level(encoder,5);FLAC__stream_encoder_set_verify(encoder,true);
  if(FLAC__stream_encoder_init_file(encoder,dst,NULL,NULL)!=FLAC__STREAM_ENCODER_INIT_STATUS_OK)ok=0;
  if(fseeko(in,(off_t)info.data_offset,SEEK_SET))ok=0;const size_t frame_bytes=(size_t)info.channels*info.bits/8;const size_t frames_per_buffer=sizeof(bytes)/frame_bytes;FLAC__int32 samples[65536];
  while(ok&&done<info.frames){size_t frames=(size_t)(info.frames-done);if(frames>frames_per_buffer)frames=frames_per_buffer;size_t count=frames*frame_bytes;if(fread(bytes,1,count,in)!=count){ok=0;break;}
   for(size_t i=0;i<frames*info.channels;i++){const unsigned char*p=bytes+i*(info.bits/8);int32_t v;if(info.bits==8)v=(int)p[0]-128;else if(info.bits==16)v=(int16_t)u16(p);else{v=(int32_t)((uint32_t)p[0]|(uint32_t)p[1]<<8|(uint32_t)p[2]<<16);if(v&0x800000)v|=(int32_t)0xff000000;}samples[i]=v;}
   if(!FLAC__stream_encoder_process_interleaved(encoder,samples,(unsigned)frames))ok=0;done+=frames;if(progress&&progress((double)done/info.frames*.6,context))ok=0;
  }
  if(!FLAC__stream_encoder_finish(encoder))ok=0;FLAC__stream_encoder_delete(encoder);
  if(ok){FILE*f=fopen(dst,"r+b");if(!f||fsync(fileno(f)))ok=0;if(f)fclose(f);}
  if(ok){Verify v={0,0};FLAC__StreamDecoder*d=FLAC__stream_decoder_new();FLAC__stream_decoder_set_md5_checking(d,true);if(FLAC__stream_decoder_init_file(d,dst,decoded,NULL,decode_error,&v)!=FLAC__STREAM_DECODER_INIT_STATUS_OK)ok=0;else{if(!FLAC__stream_decoder_process_until_end_of_stream(d)||!FLAC__stream_decoder_finish(d)||v.failed||v.frames!=info.frames)ok=0;}FLAC__stream_decoder_delete(d);}
 }
 fclose(in);if(!ok){remove(dst);return fail(err,n,"Conversion cancelled, storage full, source lost, or audio verification failed");}if(progress)progress(1.,context);return 0;
}
