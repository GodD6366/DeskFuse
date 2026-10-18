find_package(Protobuf CONFIG REQUIRED)
find_package(gRPC CONFIG REQUIRED)
add_library(roc-microphone SHARED microphone.cpp)
target_include_directories(roc-microphone PUBLIC include PRIVATE ${PROJECT_ROOT}/rpc)
target_link_libraries(roc-microphone PRIVATE roc ${RPC_LIB} gRPC::grpc++_unsecure "-framework CoreAudio" "-framework CoreFoundation")
target_link_options(roc-microphone PRIVATE "-Wl,-exported_symbol,_ud_mic_*" "-Wl,-dead_strip")
set_target_properties(roc-microphone PROPERTIES INSTALL_NAME_DIR "@rpath" BUILD_WITH_INSTALL_NAME_DIR ON)

add_custom_command(TARGET roc-microphone POST_BUILD COMMAND strip -x $<TARGET_FILE:roc-microphone>)

# Synthetic audio only: exercises the shipped sender ABI and matching receiver
# without microphone permission, HAL installation, or touching system audio.
add_executable(microphone-codec-test EXCLUDE_FROM_ALL tests/codec.cpp)
target_link_libraries(microphone-codec-test PRIVATE roc-microphone roc)
