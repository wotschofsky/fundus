#include <jni.h>
#include "vault_audio.h"

// Reports progress to AudioCodec.progress(double), which returns true to stop.
typedef struct { JNIEnv *env; jobject owner; jmethodID method; } Callback;
static int progress(double amount, void *context) {
    Callback *c = context;
    return (*c->env)->CallBooleanMethod(c->env, c->owner, c->method, amount) == JNI_TRUE;
}

JNIEXPORT jstring JNICALL Java_dev_wsky_fundus_AudioCodec_nativePrepare(JNIEnv *env, jobject self, jstring source, jstring destination) {
    const char *src = (*env)->GetStringUTFChars(env, source, 0);
    const char *dst = (*env)->GetStringUTFChars(env, destination, 0);
    char error[512] = {0};
    Callback callback = {env, self, (*env)->GetMethodID(env, (*env)->GetObjectClass(env, self), "progress", "(D)Z")};
    int result = vault_audio_prepare(src, dst, progress, &callback, error, sizeof(error));
    (*env)->ReleaseStringUTFChars(env, source, src);
    (*env)->ReleaseStringUTFChars(env, destination, dst);
    return (*env)->NewStringUTF(env, result ? error : "");
}
