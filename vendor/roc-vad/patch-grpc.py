#!/usr/bin/env python3
"""Apply audited compatibility and listener recovery patches to pinned gRPC."""
import pathlib
import sys
source=pathlib.Path(sys.argv[1])/'third_party/abseil-cpp/absl/copts/AbseilConfigureCopts.cmake'
text=source.read_text()
old='list(APPEND ABSL_RANDOM_RANDEN_COPTS "-Xarch_${_arch}" "${_flag}")'
new='list(APPEND ABSL_RANDOM_RANDEN_COPTS "SHELL:-Xarch_${_arch} ${_flag}")'
if old in text: source.write_text(text.replace(old,new))
elif new not in text: raise SystemExit('Pinned Abseil architecture flag patch does not match')
source=pathlib.Path(sys.argv[1])/'src/core/lib/promise/detail/basic_seq.h'
text=source.read_text()
old='Traits::template CallSeqFactory(f_, *cur_, std::move(arg))'
new='Traits::CallSeqFactory(f_, *cur_, std::move(arg))'
if old in text: source.write_text(text.replace(old,new))
elif new not in text: raise SystemExit('Pinned gRPC template compatibility patch does not match')

# Darwin may return ECONNRESET when a queued client closes before accept().
# It invalidates that connection, not the listening socket. Retry accept so the
# listener drains its queue instead of abandoning its read notification forever.
for relative, old, new in [
    ('src/core/lib/iomgr/tcp_server_posix.cc',
     'if (errno == EINTR) {',
     'if (errno == EINTR || errno == ECONNRESET) {'),
    ('src/core/lib/event_engine/posix_engine/posix_engine_listener.cc',
     '        case EINTR:\n          continue;',
     '        case EINTR:\n        case ECONNRESET:\n          continue;'),
]:
    source = pathlib.Path(sys.argv[1]) / relative
    text = source.read_text()
    if text.count(old) == 1:
        source.write_text(text.replace(old, new))
    elif text.count(new) != 1:
        raise SystemExit('Pinned gRPC accept reset patch does not match: ' + relative)

# On macOS, a peer that resets during accept can also yield an accepted socket
# with an empty peer address. Discard that socket, not the listening service.
for relative, old, new in [
    ('src/core/lib/iomgr/tcp_server_posix.cc',
     '              addr_uri.status().ToString().c_str());\n      goto error;',
     '              addr_uri.status().ToString().c_str());\n      close(fd);\n      continue;'),
    ('src/core/lib/event_engine/posix_engine/posix_engine_listener.cc',
     '              peer_name.status().ToString().c_str());\n      // Shutting down the acceptor. Unref the ref grabbed in\n      // AsyncConnectionAcceptor::Start().\n      Unref();\n      return;',
     '              peer_name.status().ToString().c_str());\n      close(fd);\n      continue;'),
]:
    source = pathlib.Path(sys.argv[1]) / relative
    text = source.read_text()
    if text.count(old) == 1:
        source.write_text(text.replace(old, new))
    elif text.count(new) != 1:
        raise SystemExit('Pinned gRPC peer address patch does not match: ' + relative)

# This private gRPC build carries only low-rate microphone control RPCs. Reserve
# the upstream minimum instead of one worker per CPU in the small HAL host;
# the pool can still grow on demand. Audio uses the separate Roc threads.
source = pathlib.Path(sys.argv[1]) / 'src/core/lib/event_engine/posix_engine/posix_engine.cc'
text = source.read_text()
old = 'executor_(MakeThreadPool(grpc_core::Clamp(gpr_cpu_num_cores(), 4u, 16u)))'
new = 'executor_(MakeThreadPool(4))'
if text.count(old) == 2:
    source.write_text(text.replace(old, new))
elif text.count(new) != 2:
    raise SystemExit('Pinned gRPC control worker pool patch does not match')
