cmake_minimum_required(VERSION 3.20)
project(deskfuse_windows_microphone LANGUAGES CXX)

if(NOT WIN32)
  message(FATAL_ERROR "The Windows microphone sender must be built on Windows.")
endif()
if(NOT ROC_ROOT)
  message(FATAL_ERROR "Pass -DROC_ROOT=<Roc install prefix>.")
endif()

add_library(deskfuse-roc-microphone SHARED roc_windows_sender.cpp)
target_compile_features(deskfuse-roc-microphone PRIVATE cxx_std_17)
target_include_directories(deskfuse-roc-microphone PRIVATE
  "${ROC_ROOT}/include"
  "${CMAKE_CURRENT_SOURCE_DIR}"
)
target_link_directories(deskfuse-roc-microphone PRIVATE "${ROC_ROOT}/lib")
target_link_libraries(deskfuse-roc-microphone PRIVATE roc)
set_target_properties(deskfuse-roc-microphone PROPERTIES OUTPUT_NAME "deskfuse-roc-microphone")
