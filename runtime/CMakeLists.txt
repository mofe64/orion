cmake_minimum_required(VERSION 3.16)
project(orion_runtime LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(yaml-cpp REQUIRED)

set(ORION_HARDWARE_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../ros2_ws/src/orion_hardware")
set(FTSERVO_ROOT "${ORION_HARDWARE_ROOT}/vendor/ftservo")

add_library(orion_control STATIC
  src/joint_trajectory.cpp
  src/motion_library.cpp
  src/motion_sequence.cpp
  src/pose_library.cpp
  src/state_snapshot.cpp
  "${ORION_HARDWARE_ROOT}/src/calibration.cpp"
  "${ORION_HARDWARE_ROOT}/src/ftservo_transport.cpp"
  "${ORION_HARDWARE_ROOT}/src/sts3215_driver.cpp"
  "${FTSERVO_ROOT}/src/SCS.cpp"
  "${FTSERVO_ROOT}/src/SCSerial.cpp"
  "${FTSERVO_ROOT}/src/SMS_STS.cpp"
)
target_include_directories(orion_control
  PUBLIC
    "${CMAKE_CURRENT_SOURCE_DIR}/include"
    "${ORION_HARDWARE_ROOT}/include"
  PRIVATE "${FTSERVO_ROOT}/src"
)
target_link_libraries(orion_control PUBLIC yaml-cpp)

add_executable(oriond src/oriond_main.cpp)
target_link_libraries(oriond PRIVATE orion_control)

include(CTest)
if(BUILD_TESTING)
  find_package(GTest REQUIRED)
  add_executable(
    test_sts3215_driver
    "${ORION_HARDWARE_ROOT}/test/test_sts3215_driver.cpp"
  )
  target_link_libraries(test_sts3215_driver PRIVATE orion_control GTest::gtest_main)

  add_executable(test_state_snapshot test/test_state_snapshot.cpp)
  target_link_libraries(test_state_snapshot PRIVATE orion_control GTest::gtest_main)

  add_executable(test_motion test/test_motion.cpp)
  target_compile_definitions(
    test_motion PRIVATE
    ORION_TEST_POSES_FILE="${CMAKE_CURRENT_SOURCE_DIR}/../ros2_ws/src/orion_motion/config/poses.yaml"
    ORION_TEST_MOTIONS_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../ros2_ws/src/orion_motion/motions"
  )
  target_link_libraries(test_motion PRIVATE orion_control GTest::gtest_main)

  include(GoogleTest)
  gtest_discover_tests(test_sts3215_driver)
  gtest_discover_tests(test_state_snapshot)
  gtest_discover_tests(test_motion)
endif()
