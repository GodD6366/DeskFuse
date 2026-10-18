#include "deskfuse_microphone.h"
#include <roc/context.h>
#include <roc/receiver_decoder.h>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <memory>

namespace {
void require(bool valid, const char* description) {
    if (!valid) {
        std::fprintf(stderr, "FAIL: %s (%s)\n", description, ud_mic_last_error());
        std::exit(1);
    }
}
constexpr double pi = 3.14159265358979323846;
constexpr size_t frames = 240;

void round_trip(bool drop_packets, double frequency) {
    std::unique_ptr<ud_mic_sender, decltype(&ud_mic_sender_destroy)> sender(
        ud_mic_sender_create(), ud_mic_sender_destroy);
    require(sender != nullptr, "create production sender");
    roc_context* context = nullptr;
    roc_context_config context_config{};
    context_config.max_packet_size = 4096;
    require(roc_context_open(&context_config, &context) == 0, "create decoder context");
    roc_media_encoding encoding{};
    encoding.rate = 48000;
    encoding.format = ROC_FORMAT_PCM_FLOAT32;
    encoding.channels = ROC_CHANNEL_LAYOUT_MONO;
    require(roc_context_register_encoding(context, 96, &encoding) == 0, "register wire PCM");
    roc_receiver_config config{};
    config.frame_encoding = encoding;
    config.clock_source = ROC_CLOCK_SOURCE_EXTERNAL;
    config.target_latency = 100'000'000;
    config.latency_tuner_profile = ROC_LATENCY_TUNER_PROFILE_INTACT;
    roc_receiver_decoder* decoder = nullptr;
    require(roc_receiver_decoder_open(context, &config, &decoder) == 0, "open decoder");
    const roc_interface interfaces[] = {ROC_INTERFACE_AUDIO_SOURCE, ROC_INTERFACE_AUDIO_REPAIR, ROC_INTERFACE_AUDIO_CONTROL};
    const roc_protocol protocols[] = {ROC_PROTO_RTP_RS8M_SOURCE, ROC_PROTO_RS8M_REPAIR, ROC_PROTO_RTCP};
    for (size_t i = 0; i < 3; ++i) {
        require(roc_receiver_decoder_activate(decoder, interfaces[i], protocols[i]) == 0, "activate decoder interface");
    }
    std::array<float, frames> input{}, output{};
    std::array<uint8_t, 4096> bytes{};
    size_t source_packets = 0, repair_packets = 0, dropped = 0, samples = 0;
    double energy = 0, sine = 0, cosine = 0;
    for (size_t step = 0; step < 400; ++step) {
        for (size_t i = 0; i < frames; ++i) {
            input[i] = static_cast<float>(0.4 * std::sin(2 * pi * frequency * (step * frames + i) / 48000));
        }
        require(ud_mic_sender_push(sender.get(), input.data(), input.size()) == 0, "encode frame");
        for (int i = 0; i < 3; ++i) {
            for (;;) {
                int length = ud_mic_sender_pop(sender.get(), i + 1, bytes.data(), bytes.size());
                require(length >= 0, "pop sender packet");
                if (length == 0) break;
                if (i == 0) {
                    ++source_packets;
                    // One source loss per 20 packets is inside RS8M 10/4 capacity.
                    if (drop_packets && source_packets % 20 == 0) { ++dropped; continue; }
                }
                if (i == 1) ++repair_packets;
                roc_packet packet{bytes.data(), static_cast<size_t>(length)};
                require(roc_receiver_decoder_push_packet(decoder, interfaces[i], &packet) == 0, "decode packet");
            }
        }
        roc_frame frame{output.data(), output.size() * sizeof(float)};
        require(roc_receiver_decoder_pop_frame(decoder, &frame) == 0, "read decoded frame");
        require(frame.samples_size == output.size() * sizeof(float), "decoded frame length");
        if (step >= 100) {
            for (float sample : output) {
                require(std::isfinite(sample), "finite decoded samples");
                double phase = 2 * pi * frequency * samples / 48000;
                energy += sample * sample;
                sine += sample * std::sin(phase);
                cosine += sample * std::cos(phase);
                ++samples;
            }
        }
        for (;;) {
            roc_packet feedback{bytes.data(), bytes.size()};
            if (roc_receiver_decoder_pop_feedback_packet(decoder, ROC_INTERFACE_AUDIO_CONTROL, &feedback) < 0) break;
            require(ud_mic_sender_feedback(sender.get(), bytes.data(), feedback.bytes_size) == 0, "deliver receiver feedback");
        }
    }
    double rms = std::sqrt(energy / samples);
    double tone_ratio = 2 * (sine * sine + cosine * cosine) / (samples * energy);
    require(source_packets > 300 && repair_packets > 100, "RTP and repair traffic");
    require(!drop_packets || dropped > 10, "packet loss exercised");
    require(rms > 0.26 && rms < 0.30, "decoded signal amplitude");
    require(tone_ratio > 0.98, "decoded signal frequency and continuity");
    std::printf("PASS: %.0f Hz, dropped=%zu, RMS=%.5f, tone=%.5f\n", frequency, dropped, rms, tone_ratio);
    require(roc_receiver_decoder_close(decoder) == 0, "close decoder");
    require(roc_context_close(context) == 0, "close context");
}
}
int main() {
    round_trip(false, 1000);
    round_trip(true, 600);
    // Fresh sessions must work independently after decoder/sender teardown.
    round_trip(false, 1400);
}
