# HC02: builds task1_divergence, task2_bandwidth (CUDA) and cpu_baseline (C++).
# Raw nvcc fallback lines are in README.md.

cmake_minimum_required(VERSION 3.18)

# Must be set before enabling the CUDA language. "native" needs CMake >= 3.24;
# otherwise build a fatbin for all supported architectures >= cc 7.0.
if(NOT DEFINED CMAKE_CUDA_ARCHITECTURES)
  if(CMAKE_VERSION VERSION_GREATER_EQUAL 3.24)
    set(CMAKE_CUDA_ARCHITECTURES native)
  else()
    set(CMAKE_CUDA_ARCHITECTURES 70 75 80 86 89 90)
  endif()
endif()

project(hc02 LANGUAGES CXX CUDA)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CUDA_STANDARD 17)
set(CMAKE_CUDA_STANDARD_REQUIRED ON)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

add_executable(task1_divergence src/task1_divergence.cu)
add_executable(task2_bandwidth src/task2_bandwidth.cu)
foreach(tgt task1_divergence task2_bandwidth)
  # -Wall -Wextra for the host-side code; no fast math (keep FMA counts honest).
  target_compile_options(${tgt} PRIVATE
    $<$<COMPILE_LANGUAGE:CUDA>:-Xcompiler=-Wall,-Wextra>)
endforeach()

find_package(OpenMP REQUIRED)
add_executable(cpu_baseline src/cpu_baseline.cpp)
target_link_libraries(cpu_baseline PRIVATE OpenMP::OpenMP_CXX)
target_compile_options(cpu_baseline PRIVATE -Wall -Wextra -march=native)
