#include <gtest/gtest.h>

#include <memory>

#include "hardware_interface/system_interface.hpp"
#include "pluginlib/class_loader.hpp"

TEST(OrionHardwarePluginTest, Sts3215SystemIsDiscoverable)
{
  pluginlib::ClassLoader<hardware_interface::SystemInterface> loader(
    "hardware_interface", "hardware_interface::SystemInterface");

  auto system = loader.createSharedInstance("orion_hardware/STS3215System");

  ASSERT_NE(system, nullptr);
}
