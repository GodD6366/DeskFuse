#include "unidesk_microphone.h"
#include "driver_protocol.grpc.pb.h"
#include <roc/context.h>
#include <roc/sender_encoder.h>
#include <roc/frame.h>
#include <roc/packet.h>
#include <CoreAudio/CoreAudio.h>
#include <grpcpp/grpcpp.h>
#include <chrono>
#include <cstring>
#include <memory>
#include <string>

namespace {
thread_local std::string last_error;
constexpr const char *uid = "com.unidesk.microphone.input";
int fail(const char *text) { last_error = text; return -1; }
std::shared_ptr<rvpb::RvDriver::Stub> driver() {
    static std::shared_ptr<rvpb::RvDriver::Stub> stub = rvpb::RvDriver::NewStub(
        grpc::CreateChannel("127.0.0.1:19712", grpc::InsecureChannelCredentials()));
    return stub;
}
void deadline(grpc::ClientContext& c, int milliseconds = 350) {
    c.set_deadline(std::chrono::system_clock::now() + std::chrono::milliseconds(milliseconds));
}
int status(const grpc::Status& s) {
    if (s.ok()) { last_error.clear(); return 0; }
    last_error = s.error_message(); return -1;
}
}
struct ud_mic_sender {
    roc_context *context = nullptr;
    roc_sender_encoder *encoder = nullptr;
    ~ud_mic_sender() {
        if (encoder) roc_sender_encoder_close(encoder);
        if (context) roc_context_close(context);
    }
};
extern "C" {
const char *ud_mic_last_error() { return last_error.c_str(); }
ud_mic_sender *ud_mic_sender_create() {
    auto s = std::make_unique<ud_mic_sender>();
    roc_context_config context{};
    context.max_packet_size = 4096;
    if (roc_context_open(&context, &s->context) < 0) { fail("Roc context creation failed"); return nullptr; }
    roc_media_encoding encoding{};
    // Pinned Roc API maps FLOAT32 to network L16 in register_encoding(),
    // matching Roc VAD sample_format_map (roc-toolkit issue 608).
    encoding.rate = 48000; encoding.format = ROC_FORMAT_PCM_FLOAT32;
    encoding.channels = ROC_CHANNEL_LAYOUT_MONO;
    if (roc_context_register_encoding(s->context, 96, &encoding) < 0) { fail("Roc packet encoding registration failed"); return nullptr; }
    roc_sender_config config{};
    config.frame_encoding.rate = 48000;
    config.frame_encoding.format = ROC_FORMAT_PCM_FLOAT32;
    config.frame_encoding.channels = ROC_CHANNEL_LAYOUT_MONO;
    config.packet_encoding = static_cast<roc_packet_encoding>(96);
    config.packet_length = 5'000'000;
    config.fec_encoding = ROC_FEC_ENCODING_RS8M;
    config.fec_block_source_packets = 10;
    config.fec_block_repair_packets = 4;
    config.clock_source = ROC_CLOCK_SOURCE_EXTERNAL;
    if (roc_sender_encoder_open(s->context, &config, &s->encoder) < 0 ||
        roc_sender_encoder_activate(s->encoder, ROC_INTERFACE_AUDIO_SOURCE, ROC_PROTO_RTP_RS8M_SOURCE) < 0 ||
        roc_sender_encoder_activate(s->encoder, ROC_INTERFACE_AUDIO_REPAIR, ROC_PROTO_RS8M_REPAIR) < 0 ||
        roc_sender_encoder_activate(s->encoder, ROC_INTERFACE_AUDIO_CONTROL, ROC_PROTO_RTCP) < 0) {
        fail("Roc sender encoder initialization failed"); return nullptr;
    }
    return s.release();
}
int ud_mic_sender_push(ud_mic_sender *s, const float *samples, size_t frames) {
    if (!s || !samples || frames > 48000) return fail("Invalid microphone frame");
    roc_frame frame{}; frame.samples = const_cast<float *>(samples); frame.samples_size = frames * sizeof(float);
    return roc_sender_encoder_push_frame(s->encoder, &frame);
}
int ud_mic_sender_pop(ud_mic_sender *s, int interface_id, uint8_t *bytes, size_t capacity) {
    if (!s || !bytes || capacity < 4096 || interface_id < 1 || interface_id > 3) return fail("Invalid microphone packet buffer");
    roc_packet packet{}; packet.bytes = bytes; packet.bytes_size = capacity;
    if (roc_sender_encoder_pop_packet(s->encoder, static_cast<roc_interface>(interface_id + 10), &packet) < 0) return 0;
    return static_cast<int>(packet.bytes_size);
}
int ud_mic_sender_feedback(ud_mic_sender *s, const uint8_t *bytes, size_t length) {
    if (!s || !bytes || length > 4096) return fail("Invalid microphone feedback");
    roc_packet packet{}; packet.bytes = const_cast<uint8_t *>(bytes); packet.bytes_size = length;
    return roc_sender_encoder_push_feedback_packet(s->encoder, ROC_INTERFACE_AUDIO_CONTROL, &packet);
}
void ud_mic_sender_destroy(ud_mic_sender *s) { delete s; }
int ud_mic_driver_available() {
    grpc::ClientContext c; deadline(c, 150); rvpb::RvNone in, out;
    return status(driver()->ping(&c, in, &out)) == 0 ? 1 : -1;
}
int ud_mic_driver_set_enabled(int enabled) {
    grpc::ClientContext c; deadline(c); rvpb::RvToggleRequest in; rvpb::RvNone out;
    in.mutable_device()->set_uid(uid); in.set_enabled(enabled != 0);
    return status(driver()->toggle_device(&c, in, &out));
}
int ud_mic_driver_reset() {
    grpc::ClientContext c; deadline(c); rvpb::RvNone in, out;
    return status(driver()->reset_receiver(&c, in, &out));
}
int ud_mic_driver_configure(const char *name) {
    if (!name || !*name || std::strlen(name) > 512) return fail("Invalid microphone display name");
    auto stub = driver(); rvpb::RvDeviceSelector selector; selector.set_uid(uid);
    rvpb::RvDeviceInfo existing;
    { grpc::ClientContext c; deadline(c);
      auto result = stub->get_device(&c, selector, &existing);
      if (result.ok() && existing.name() == name) return ud_mic_driver_set_enabled(1);
      if (result.ok()) {
          grpc::ClientContext d; deadline(d); rvpb::RvNone out;
          if (status(stub->delete_device(&d, selector, &out)) < 0) return -1;
      } else if (result.error_code() == grpc::UNAVAILABLE || result.error_code() == grpc::DEADLINE_EXCEEDED) return status(result);
    }
    rvpb::RvDeviceInfo in, out; in.set_uid(uid); in.set_name(name);
    in.set_type(rvpb::RV_DEVICE_TYPE_RECEIVER); in.set_enabled(true);
    auto *dev = in.mutable_device_encoding(); dev->set_sample_rate(48000);
    dev->set_channel_layout(rvpb::RV_CHANNEL_LAYOUT_MONO);
    dev->mutable_buffer_length()->set_nanos(60'000'000);
    auto *config = in.mutable_receiver_config();
    config->mutable_target_latency()->set_nanos(100'000'000);
    config->mutable_latency_tolerance()->set_nanos(100'000'000);
    config->mutable_no_playback_timeout()->set_seconds(1);
    config->mutable_choppy_playback_timeout()->set_seconds(1);
    auto *encoding = config->add_packet_encodings(); encoding->set_encoding_id(96);
    encoding->set_sample_rate(48000); encoding->set_sample_format(rvpb::ROC_FORMAT_PCM_SINT16);
    encoding->set_channel_layout(rvpb::RV_CHANNEL_LAYOUT_MONO);
    const char *uris[] = {"rtp+rs8m://127.0.0.1:19721", "rs8m://127.0.0.1:19722", "rtcp://127.0.0.1:19723"};
    for (int i=0; i<3; ++i) { auto *e=in.add_local_endpoints(); e->set_slot(0); e->set_interface(static_cast<rvpb::RvInterface>(i+1)); e->set_uri(uris[i]); }
    grpc::ClientContext c; deadline(c); return status(stub->add_device(&c, in, &out));
}
int ud_mic_driver_has_readers() {
    grpc::ClientContext c; deadline(c); rvpb::RvNone in; rvpb::RvReaderState out;
    if (status(driver()->receiver_readers(&c, in, &out)) < 0) return -1;
    return out.reading() ? 1 : 0;
}
}
