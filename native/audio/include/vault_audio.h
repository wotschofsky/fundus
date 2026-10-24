#ifndef VAULT_AUDIO_H
#define VAULT_AUDIO_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { uint32_t sample_rate; uint16_t channels; uint16_t bits; uint16_t is_float; uint64_t frames; uint64_t data_offset; uint64_t data_bytes; int flac; } VaultAudioInfo;
typedef int (*VaultProgress)(double fraction, void *context);
int vault_audio_inspect(const char *source, VaultAudioInfo *info, char *error, int error_size);
int vault_audio_prepare(const char *source, const char *destination, VaultProgress progress, void *context, char *error, int error_size);
#ifdef __cplusplus
}
#endif
#endif
