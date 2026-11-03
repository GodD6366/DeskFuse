#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct deskfuse_roc_sender deskfuse_roc_sender;
typedef struct deskfuse_roc_receiver deskfuse_roc_receiver;

deskfuse_roc_sender *deskfuse_roc_sender_create(void);
int deskfuse_roc_sender_push_f32(deskfuse_roc_sender *, const float *, size_t frames);
int deskfuse_roc_sender_pop(deskfuse_roc_sender *, int interface_id, uint8_t *, size_t capacity);
int deskfuse_roc_sender_feedback(deskfuse_roc_sender *, const uint8_t *, size_t bytes);
void deskfuse_roc_sender_destroy(deskfuse_roc_sender *);
const char *deskfuse_roc_sender_last_error(void);

deskfuse_roc_receiver *deskfuse_roc_receiver_create(void);
int deskfuse_roc_receiver_push(deskfuse_roc_receiver *, int interface_id, const uint8_t *, size_t bytes);
int deskfuse_roc_receiver_pop_f32(deskfuse_roc_receiver *, float *, size_t frames);
int deskfuse_roc_receiver_pop_feedback(deskfuse_roc_receiver *, uint8_t *, size_t capacity);
void deskfuse_roc_receiver_destroy(deskfuse_roc_receiver *);

#ifdef __cplusplus
}
#endif
