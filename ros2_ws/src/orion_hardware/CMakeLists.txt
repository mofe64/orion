cmake_minimum_required(VERSION 3.16)
project(orion_hardware)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(ament_cmake REQUIRED)
find_package(hardware_interface REQUIRED)
find_package(pluginlib REQUIRED)
find_package(rclcpp REQUIRED)
find_package(rclcpp_lifecycle REQUIRED)
find_package(yaml_cpp_vendor REQUIRED)

add_library(orion_hardware SHARED
  src/calibration.cpp
  src/ftservo_transport.cpp
  src/sts3215_driver.cpp
  src/sts3215_system.cpp
  vendor/ftservo/src/SCS.cpp
  vendor/ftservo/src/SCSerial.cpp
  vendor/ftservo/src/SMS_STS.cpp
)
target_include_directories(orion_hardware PUBLIC
  $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
  $<INSTALL_INTERFACE:include>
)
target_include_directories(orion_hardware PRIVATE vendor/ftservo/src)
ament_target_dependencies(orion_hardware
  hardware_interface
  pluginlib
  rclcpp
  rclcpp_lifecycle
  yaml_cpp_vendor
)
target_link_libraries(orion_hardware yaml-cpp)

pluginlib_export_plugin_description_file(
  hardware_interface
  orion_hardware_plugins.xml
)

install(
  DIRECTORY include/
  DESTINATION include
)
install(
  TARGETS orion_hardware
  EXPORT export_orion_hardware
  ARCHIVE DESTINATION lib
  LIBRARY DESTINATION lib
  RUNTIME DESTINATION bin
)

if(BUILD_TESTING)
  find_package(ament_cmake_gtest REQUIRED)

  ament_add_gtest(test_sts3215_driver test/test_sts3215_driver.cpp)
  target_link_libraries(test_sts3215_driver orion_hardware)

  ament_add_gtest(test_plugin_load test/test_plugin_load.cpp)
  target_link_libraries(test_plugin_load orion_hardware)
  ament_target_dependencies(test_plugin_load hardware_interface pluginlib)
endif()

ament_export_targets(export_orion_hardware HAS_LIBRARY_TARGET)
ament_export_dependencies(
  hardware_interface
  pluginlib
  rclcpp
  rclcpp_lifecycle
  yaml_cpp_vendor
)
ament_package()
