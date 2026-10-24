#include <jni.h>
#include "vault_audio.h"
typedef struct { JNIEnv *env; jobject owner; jmethodID method; } Cancel;
static int progress(double amount, void *context) { (void)amount; Cancel *c=context; return (*c->env)->CallBooleanMethod(c->env,c->owner,c->method) == JNI_TRUE; }
JNIEXPORT jstring JNICALL Java_dev_wsky_vaulttransfer_AudioCodec_prepare(JNIEnv* env,jobject self,jstring source,jstring destination){
 const char*src=(*env)->GetStringUTFChars(env,source,0);const char*dst=(*env)->GetStringUTFChars(env,destination,0);char error[512]={0};
 Cancel cancel={env,self,(*env)->GetMethodID(env,(*env)->GetObjectClass(env,self),"isCancelled","()Z")};
 int result=vault_audio_prepare(src,dst,progress,&cancel,error,sizeof(error));
 (*env)->ReleaseStringUTFChars(env,source,src);(*env)->ReleaseStringUTFChars(env,destination,dst);
 return (*env)->NewStringUTF(env,result?error:"");
}
JNIEXPORT jboolean JNICALL Java_dev_wsky_vaulttransfer_AudioCodec_canCompress(JNIEnv* env,jobject self,jstring source){
 const char*src=(*env)->GetStringUTFChars(env,source,0);VaultAudioInfo info;char error[512];int result=vault_audio_inspect(src,&info,error,sizeof(error));(*env)->ReleaseStringUTFChars(env,source,src);
 if(result){jclass c=(*env)->FindClass(env,"java/io/IOException");(*env)->ThrowNew(env,c,error);return JNI_FALSE;}return info.flac?JNI_TRUE:JNI_FALSE;
}
