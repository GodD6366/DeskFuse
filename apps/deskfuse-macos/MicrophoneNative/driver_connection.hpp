#pragma once
#include "driver_protocol.grpc.pb.h"
#include <grpcpp/grpcpp.h>
#include <chrono>
#include <memory>
#include <mutex>
#include <string>

// A failed health check must not keep reusing a stale channel after HAL restarts.
// Only the read-only ping is retried; mutating RPCs are never replayed here.
class MicrophoneDriverConnection {
public:
    explicit MicrophoneDriverConnection(std::string endpoint) : endpoint_(std::move(endpoint)) {}
    std::shared_ptr<rvpb::RvDriver::Stub> stub() {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!stub_) {
            grpc::ChannelArguments args;
            args.SetInt("grpc.use_local_subchannel_pool", 1);
            args.SetInt("grpc.enable_http_proxy", 0);
            stub_ = rvpb::RvDriver::NewStub(grpc::CreateCustomChannel(
                endpoint_, grpc::InsecureChannelCredentials(), args));
        }
        return stub_;
    }
    grpc::Status ping() {
        grpc::Status result;
        for (int attempt = 0; attempt < 2; ++attempt) {
            auto current = stub();
            grpc::ClientContext context;
            context.set_deadline(std::chrono::system_clock::now() + std::chrono::milliseconds(350));
            rvpb::RvNone request, response;
            result = current->ping(&context, request, &response);
            if (result.ok()) return result;
            if (result.error_code() != grpc::UNAVAILABLE && result.error_code() != grpc::DEADLINE_EXCEEDED)
                return result;
            std::lock_guard<std::mutex> lock(mutex_);
            if (stub_ == current) stub_.reset();
        }
        return result;
    }
private:
    const std::string endpoint_;
    std::mutex mutex_;
    std::shared_ptr<rvpb::RvDriver::Stub> stub_;
};
