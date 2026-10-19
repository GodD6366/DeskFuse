// Isolated control-service regression. Never loads HAL or opens a microphone.
#include "../driver_connection.hpp"
#include <grpcpp/grpcpp.h>
#include <atomic>
#include <chrono>
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <dlfcn.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>

extern char** environ;

namespace {
std::atomic<bool> reset_next_accept{false};
std::atomic<unsigned> injected{0};
std::atomic<bool> empty_next_address{false};
std::atomic<unsigned> empty_injected{0};
int accept_with_reset(int fd, sockaddr* address, socklen_t* length) {
    if (reset_next_accept.exchange(false)) {
        ++injected;
        errno = ECONNRESET;
        return -1;
    }
    using Accept = int (*)(int, sockaddr*, socklen_t*);
    static auto original = reinterpret_cast<Accept>(dlsym(RTLD_NEXT, "accept"));
    int client = original(fd, address, length);
    if (client >= 0 && empty_next_address.exchange(false)) {
        *length = 0;
        ++empty_injected;
    }
    return client;
}
class Service final : public rvpb::RvDriver::Service {
public:
    std::atomic<bool> timeout_next_ping{false};
    grpc::Status ping(grpc::ServerContext*, const rvpb::RvNone*, rvpb::RvNone*) override {
        if (timeout_next_ping.exchange(false))
            return grpc::Status(grpc::DEADLINE_EXCEEDED, "injected timeout");
        return grpc::Status::OK;
    }
};
std::unique_ptr<rvpb::RvDriver::Stub> connect_stub(const std::string& endpoint) {
    grpc::ChannelArguments args;
    args.SetInt("grpc.use_local_subchannel_pool", 1);
    args.SetInt("grpc.enable_http_proxy", 0);
    return rvpb::RvDriver::NewStub(grpc::CreateCustomChannel(
        endpoint, grpc::InsecureChannelCredentials(), args));
}
bool ping(rvpb::RvDriver::Stub& stub) {
    grpc::ClientContext context;
    context.set_deadline(std::chrono::system_clock::now() + std::chrono::milliseconds(700));
    rvpb::RvNone request, response;
    auto status = stub.ping(&context, request, &response);
    if (!status.ok()) std::fprintf(stderr, "ping: %s\n", status.error_message().c_str());
    return status.ok();
}
bool ping(const std::string& endpoint) {
    return ping(*connect_stub(endpoint));
}
void require(bool value, const char* message) {
    if (!value) { std::fprintf(stderr, "FAIL: %s\n", message); std::_Exit(1); }
}
}
// Override only this test executable's accept symbol. The statically linked
// production gRPC listener calls it; RTLD_NEXT reaches the real system call.
extern "C" int accept(int fd, sockaddr* address, socklen_t* length) {
    return accept_with_reset(fd, address, length);
}
int main(int argc, char** argv) {
    if (argc == 3) {
        // A separate process holds a channel across requests, like the app.
        // _Exit deliberately skips gRPC teardown in the abrupt-exit variant.
        auto stub = connect_stub(argv[2]);
        for (int i = 0; i < 10; ++i) require(ping(*stub), "child RPC");
        if (std::string(argv[1]) == "--abrupt-client") std::_Exit(0);
        return 0;
    }
    Service service;
    grpc::ServerBuilder builder;
    int port = 0;
    builder.AddListeningPort("127.0.0.1:0", grpc::InsecureServerCredentials(), &port);
    builder.RegisterService(&service);
    auto server = builder.BuildAndStart();
    require(server && port, "start isolated listener");
    const std::string endpoint = "127.0.0.1:" + std::to_string(port);
    require(ping(endpoint), "initial connection");
    reset_next_accept = true;
    // The triggering attempt may succeed after a transparent connection retry.
    ping(endpoint);
    require(injected == 1, "ECONNRESET reached the real listener");
    require(ping(endpoint), "new client must connect after accept ECONNRESET");
    empty_next_address = true;
    ping(endpoint);
    require(empty_injected == 1, "empty peer address reached the real listener");
    require(ping(endpoint), "new client must connect after an empty peer address");
    for (int i = 0; i < 40; ++i) {
        int fd = socket(AF_INET, SOCK_STREAM, 0);
        require(fd >= 0, "create resetting client");
        linger immediate{1, 0};
        require(setsockopt(fd, SOL_SOCKET, SO_LINGER, &immediate, sizeof(immediate)) == 0, "enable abrupt client exit");
        sockaddr_in address{};
        address.sin_family = AF_INET;
        address.sin_port = htons(port);
        address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        require(connect(fd, reinterpret_cast<sockaddr*>(&address), sizeof(address)) == 0, "connect resetting client");
        close(fd);
        require(ping(endpoint), "RPC still works after abrupt client exit");
    }
    for (int i = 0; i < 20; ++i) {
        char* child_args[] = {argv[0], const_cast<char*>(i % 2 ? "--abrupt-client" : "--client"),
                             const_cast<char*>(endpoint.c_str()), nullptr};
        pid_t child = 0;
        require(posix_spawn(&child, argv[0], nullptr, nullptr, child_args, environ) == 0,
                "start independent client process");
        int result = 0;
        pid_t waited;
        do { waited = waitpid(child, &result, 0); } while (waited < 0 && errno == EINTR);
        require(waited == child && WIFEXITED(result) && WEXITSTATUS(result) == 0,
                "independent client requests and exit");
        require(ping(endpoint), "RPC still works after established client process exits");
    }
    MicrophoneDriverConnection persistent(endpoint);
    require(persistent.ping().ok(), "production client initial ping");
    service.timeout_next_ping = true;
    require(persistent.ping().ok(), "production client retries a transient timeout");
    require(!service.timeout_next_ping, "timeout injection was consumed");
    server->Shutdown();
    server->Wait();
    require(!persistent.ping().ok(), "offline driver is not reported ready");
    server.reset();
    grpc::ServerBuilder restarted;
    restarted.AddListeningPort(endpoint, grpc::InsecureServerCredentials());
    restarted.RegisterService(&service);
    server = restarted.BuildAndStart();
    require(server != nullptr, "restart isolated driver at same endpoint");
    require(persistent.ping().ok(), "production client recovers without manual reconnect");
    server->Shutdown();
    std::puts("PASS: production client automatically reconnects after driver restart");
    std::puts("PASS: accept reset / empty address recovery, 40 abrupt TCP reconnects, 20 client process exits");
}
