#include "roc_windows_sender.h"

#include <roc/context.h>
#include <roc/frame.h>
#include <roc/packet.h>
#include <roc/sender_encoder.h>
#include <roc/receiver_decoder.h>

#include <memory>
#include <string>

namespace {
thread_local std::string last_error;
int fail(const char *message) {
    last_error = message;
    return -1;
}
}

struct deskfuse_roc_sender {
    roc_context *context = nullptr;
    roc_sender_encoder *encoder = nullptr;
    ~deskfuse_roc_sender() {
        if (encoder) roc_sender_encoder_close(encoder);
        if (context) roc_context_close(context);
    }
};

struct deskfuse_roc_receiver {
    roc_context *context = nullptr;
    roc_receiver_decoder *decoder = nullptr;
    ~deskfuse_roc_receiver() {
        if (decoder) roc_receiver_decoder_close(decoder);
        if (context) roc_context_close(context);
    }
};

extern "C" {
const char *deskfuse_roc_sender_last_error() { return last_error.c_str(); }

deskfuse_roc_sender *deskfuse_roc_sender_create() {
    auto sender = std::make_unique<deskfuse_roc_sender>();
    roc_context_config context_config{};
    context_config.max_packet_size = 4096;
    if (roc_context_open(&context_config, &sender->context) < 0) {
        fail("Roc context creation failed");
        return nullptr;
    }
    roc_media_encoding encoding{};
    encoding.rate = 48000;
    encoding.format = ROC_FORMAT_PCM_FLOAT32;
    encoding.channels = ROC_CHANNEL_LAYOUT_MONO;
    if (roc_context_register_encoding(sender->context, 96, &encoding) < 0) {
        fail("Roc packet encoding registration failed");
        return nullptr;
    }
    roc_sender_config config{};
    config.frame_encoding = encoding;
    config.packet_encoding = static_cast<roc_packet_encoding>(96);
    config.packet_length = 5'000'000;
    config.fec_encoding = ROC_FEC_ENCODING_RS8M;
    config.fec_block_source_packets = 10;
    config.fec_block_repair_packets = 4;
    config.clock_source = ROC_CLOCK_SOURCE_EXTERNAL;
    if (roc_sender_encoder_open(sender->context, &config, &sender->encoder) < 0 ||
        roc_sender_encoder_activate(sender->encoder, ROC_INTERFACE_AUDIO_SOURCE, ROC_PROTO_RTP_RS8M_SOURCE) < 0 ||
        roc_sender_encoder_activate(sender->encoder, ROC_INTERFACE_AUDIO_REPAIR, ROC_PROTO_RS8M_REPAIR) < 0 ||
        roc_sender_encoder_activate(sender->encoder, ROC_INTERFACE_AUDIO_CONTROL, ROC_PROTO_RTCP) < 0) {
        fail("Roc sender encoder initialization failed");
        return nullptr;
    }
    last_error.clear();
    return sender.release();
}

int deskfuse_roc_sender_push_f32(deskfuse_roc_sender *sender, const float *samples, size_t frames) {
    if (!sender || !samples || frames == 0 || frames > 48000) return fail("Invalid microphone frame");
    roc_frame frame{};
    frame.samples = const_cast<float *>(samples);
    frame.samples_size = frames * sizeof(float);
    return roc_sender_encoder_push_frame(sender->encoder, &frame);
}

int deskfuse_roc_sender_pop(deskfuse_roc_sender *sender, int interface_id, uint8_t *bytes, size_t capacity) {
    if (!sender || !bytes || capacity < 4096 || interface_id < 1 || interface_id > 3) return fail("Invalid packet buffer");
    roc_packet packet{};
    packet.bytes = bytes;
    packet.bytes_size = capacity;
    if (roc_sender_encoder_pop_packet(sender->encoder, static_cast<roc_interface>(interface_id + 10), &packet) < 0) return 0;
    return static_cast<int>(packet.bytes_size);
}

int deskfuse_roc_sender_feedback(deskfuse_roc_sender *sender, const uint8_t *bytes, size_t length) {
    if (!sender || !bytes || length > 4096) return fail("Invalid feedback packet");
    roc_packet packet{};
    packet.bytes = const_cast<uint8_t *>(bytes);
    packet.bytes_size = length;
    return roc_sender_encoder_push_feedback_packet(sender->encoder, ROC_INTERFACE_AUDIO_CONTROL, &packet);
}

void deskfuse_roc_sender_destroy(deskfuse_roc_sender *sender) { delete sender; }

deskfuse_roc_receiver *deskfuse_roc_receiver_create() {
    auto receiver = std::make_unique<deskfuse_roc_receiver>();
    roc_context_config context_config{};
    context_config.max_packet_size = 4096;
    if (roc_context_open(&context_config, &receiver->context) < 0) {
        fail("Roc receiver context creation failed");
        return nullptr;
    }
    roc_media_encoding encoding{};
    encoding.rate = 48000;
    encoding.format = ROC_FORMAT_PCM_FLOAT32;
    encoding.channels = ROC_CHANNEL_LAYOUT_MONO;
    if (roc_context_register_encoding(receiver->context, 96, &encoding) < 0) {
        fail("Roc receiver encoding registration failed");
        return nullptr;
    }
    roc_receiver_config config{};
    config.frame_encoding = encoding;
    config.clock_source = ROC_CLOCK_SOURCE_EXTERNAL;
    config.target_latency = 100'000'000;
    config.latency_tolerance = 100'000'000;
    if (roc_receiver_decoder_open(receiver->context, &config, &receiver->decoder) < 0 ||
        roc_receiver_decoder_activate(receiver->decoder, ROC_INTERFACE_AUDIO_SOURCE, ROC_PROTO_RTP_RS8M_SOURCE) < 0 ||
        roc_receiver_decoder_activate(receiver->decoder, ROC_INTERFACE_AUDIO_REPAIR, ROC_PROTO_RS8M_REPAIR) < 0 ||
        roc_receiver_decoder_activate(receiver->decoder, ROC_INTERFACE_AUDIO_CONTROL, ROC_PROTO_RTCP) < 0) {
        fail("Roc receiver decoder initialization failed");
        return nullptr;
    }
    last_error.clear();
    return receiver.release();
}

int deskfuse_roc_receiver_push(deskfuse_roc_receiver *receiver, int interface_id, const uint8_t *bytes, size_t length) {
    if (!receiver || !bytes || length == 0 || interface_id < 1 || interface_id > 3) return fail("Invalid receiver packet");
    roc_packet packet{};
    packet.bytes = const_cast<uint8_t *>(bytes);
    packet.bytes_size = length;
    return roc_receiver_decoder_push_packet(receiver->decoder, static_cast<roc_interface>(interface_id + 10), &packet);
}

int deskfuse_roc_receiver_pop_f32(deskfuse_roc_receiver *receiver, float *samples, size_t frames) {
    if (!receiver || !samples || frames == 0) return fail("Invalid receiver frame");
    roc_frame frame{};
    frame.samples = samples;
    frame.samples_size = frames * sizeof(float);
    return roc_receiver_decoder_pop_frame(receiver->decoder, &frame);
}

int deskfuse_roc_receiver_pop_feedback(deskfuse_roc_receiver *receiver, uint8_t *bytes, size_t capacity) {
    if (!receiver || !bytes || capacity < 4096) return fail("Invalid feedback buffer");
    roc_packet packet{};
    packet.bytes = bytes;
    packet.bytes_size = capacity;
    if (roc_receiver_decoder_pop_feedback_packet(receiver->decoder, ROC_INTERFACE_AUDIO_CONTROL, &packet) < 0) return 0;
    return static_cast<int>(packet.bytes_size);
}

void deskfuse_roc_receiver_destroy(deskfuse_roc_receiver *receiver) { delete receiver; }
}
