cmake_minimum_required(VERSION 3.8)
project(orion_description)

find_package(ament_cmake REQUIRED)

install(
  DIRECTORY config launch meshes rviz urdf
  DESTINATION share/${PROJECT_NAME}
  PATTERN "__pycache__" EXCLUDE
)

# Keep one source copy of Orion's native MJCF model while making it available
# through the installed ROS package share directory.
install(
  FILES
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../simulation/mujoco/robot.xml
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../simulation/mujoco/scene.xml
  DESTINATION share/${PROJECT_NAME}/mujoco
)
install(
  DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/../../../simulation/mujoco/assets
  DESTINATION share/${PROJECT_NAME}/mujoco
)

if(BUILD_TESTING)
  find_package(ament_cmake_pytest REQUIRED)
  ament_add_pytest_test(
    test_mujoco_description
    test/test_mujoco_description.py
  )
  ament_add_pytest_test(
    test_gazebo_description
    test/test_gazebo_description.py
  )
  ament_add_pytest_test(
    test_hardware_description
    test/test_hardware_description.py
  )
endif()

ament_package()
