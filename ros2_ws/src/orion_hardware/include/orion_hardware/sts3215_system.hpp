#ifndef ORION_HARDWARE__STS3215_SYSTEM_HPP_
#define ORION_HARDWARE__STS3215_SYSTEM_HPP_

#include <memory>
#include <string>
#include <vector>

#include "hardware_interface/handle.hpp"
#include "hardware_interface/system_interface.hpp"
#include "orion_hardware/sts3215_driver.hpp"
#include "rclcpp_lifecycle/state.hpp"

namespace orion_hardware
{

class STS3215System : public hardware_interface::SystemInterface
{
public:
  STS3215System();
  explicit STS3215System(std::shared_ptr<Sts3215Transport> transport);

  hardware_interface::CallbackReturn on_init(
    const hardware_interface::HardwareComponentInterfaceParams & params) override;
  hardware_interface::CallbackReturn on_configure(
    const rclcpp_lifecycle::State & previous_state) override;
  hardware_interface::CallbackReturn on_activate(
    const rclcpp_lifecycle::State & previous_state) override;
  hardware_interface::CallbackReturn on_deactivate(
    const rclcpp_lifecycle::State & previous_state) override;
  hardware_interface::CallbackReturn on_cleanup(
    const rclcpp_lifecycle::State & previous_state) override;
  hardware_interface::CallbackReturn on_error(
    const rclcpp_lifecycle::State & previous_state) override;

  hardware_interface::return_type read(
    const rclcpp::Time & time, const rclcpp::Duration & period) override;
  hardware_interface::return_type write(
    const rclcpp::Time & time, const rclcpp::Duration & period) override;

private:
  void cache_interface_handles();
  void publish_states(const std::vector<JointState> & states, bool seed_commands);

  std::shared_ptr<Sts3215Transport> transport_;
  std::unique_ptr<Sts3215Driver> driver_;
  std::string port_;
  int baud_rate_ = 1000000;
  std::vector<JointCalibration> pending_calibrations_;
  std::vector<hardware_interface::StateInterface::SharedPtr> position_state_handles_;
  std::vector<hardware_interface::StateInterface::SharedPtr> velocity_state_handles_;
  std::vector<hardware_interface::CommandInterface::SharedPtr> position_command_handles_;
};

}  // namespace orion_hardware

#endif  // ORION_HARDWARE__STS3215_SYSTEM_HPP_
