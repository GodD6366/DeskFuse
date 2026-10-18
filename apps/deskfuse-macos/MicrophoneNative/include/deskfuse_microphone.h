#ifndef DESKFUSE_MICROPHONE_H
#define DESKFUSE_MICROPHONE_H
#include <stddef.h>
#include <stdint.h>
#pragma GCC visibility push(default)
#ifdef __cplusplus
extern "C" {
#endif
/* Calls are serialized on the audio worker. No call stores samples on disk.
 * Returns 0 on success, negative on failure. pop returns bytes, 0 if empty.
 * Interface numbers: 1=source, 2=repair, 3=control. Packet capacity >=4096.
 */
typedef struct ud_mic_sender ud_mic_sender;
ud_mic_sender *ud_mic_sender_create(void);
int ud_mic_sender_push(ud_mic_sender *, const float *mono48k, size_t frames);
int ud_mic_sender_pop(ud_mic_sender *, int interface_id, uint8_t *, size_t capacity);
int ud_mic_sender_feedback(ud_mic_sender *, const uint8_t *, size_t bytes);
void ud_mic_sender_destroy(ud_mic_sender *);
/* Blocking, bounded local RPC calls; use a background worker. */
/* 1=available, -1=unavailable; other control methods return0 on success. */
int ud_mic_driver_available(void);
int ud_mic_driver_configure(const char *display_name);
int ud_mic_driver_set_enabled(int enabled);
int ud_mic_driver_reset(void);
/* 1=at least one actual HAL reader, 0=no readers, -1=not present. */
int ud_mic_driver_has_readers(void);
const char *ud_mic_last_error(void);
/* Stable compatibility identities; no RPC or audio access. */
const char *ud_mic_device_uid(void);
const char *ud_mic_driver_bundle_name(void);
#ifdef __cplusplus
}
#endif
#pragma GCC visibility pop
#endif
