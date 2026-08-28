#include "orion_hardware/sts3215_system.hpp"

#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "orion_hardware/ftservo_transport.hpp"
#include "hardware_interface/types/hardware_interface_type_values.hpp"
#include "pluginlib/class_list_macros.hpp"
#include "rclcpp/logging.hpp"

namespace orion_hardware
{
namespace
{

using CallbackReturn = hardware_interface::CallbackReturn;

bool has_interface(
  const std::vector<hardware_interface::InterfaceInfo> & interfaces,
  const std::string & name)
{
  for (const auto & interface : interfaces)
  {
    if (interface.name == name)
    {
      return true;
    }
  }
  return false;
}

}  // namespace

STS3215System::STS3215System()
: STS3215System(std::make_shared<FtServoTransport>())
{
}

STS3215System::STS3215System(std::shared_ptr<Sts3215Transport> transport)
: transport_(std::move(transport))
{
}

CallbackReturn STS3215System::on_init(
  const hardware_interface::HardwareComponentInterfaceParams & params)
{
  if (hardware_interface::SystemInterface::on_init(params) != CallbackReturn::SUCCESS)
  {
    return CallbackReturn::ERROR;
  }

  try
  {
    const auto & info = get_hardware_info();
    if (info.joints.size() != 5)
    {
      throw std::runtime_error("Orion STS3215 hardware requires exactly five joints.");
    }

    std::vector<std::string> joint_names;
    joint_names.reserve(info.joints.size());
    for (const auto & joint : info.joints)
    {
      if (joint.command_interfaces.size() != 1 ||
        joint.command_interfaces.front().name != hardware_interface::HW_IF_POSITION)
      {
        throw std::runtime_error(joint.name + " must expose one position command interface.");
      }
      if (!has_interface(joint.state_interfaces, hardware_interface::HW_IF_POSITION) ||
        !has_interface(joint.state_interfaces, hardware_interface::HW_IF_VELOCITY))
      {
        throw std::runtime_error(joint.name + " must expose position and velocity state.");
      }
      joint_names.push_back(joint.name);
    }

    const auto port = info.hardware_parameters.find("port");
    const auto calibration = info.hardware_parameters.find("calibration_file");
    if (port == info.hardware_parameters.end() || port->second.empty())
    {
      throw std::runtime_error("Missing ros2_control hardware parameter: port.");
    }
    if (calibration == info.hardware_parameters.end() || calibration->second.empty())
    {
      throw std::runtime_error("Missing ros2_control hardware parameter: calibration_file.");
    }
    port_ = port->second;

    const auto baud = info.hardware_parameters.find("baud_rate");
    if (baud != info.hardware_parameters.end())
    {
      baud_rate_ = std::stoi(baud->second);
    }
    if (baud_rate_ <= 0)
    {
      throw std::runtime_error("baud_rate must be positive.");
    }

    auto calibrations = load_calibration_file(calibration->second, joint_names);
    driver_ = std::make_unique<Sts3215Driver>(transport_);
    // configure() owns both transport setup and the profile, so retain the parsed values here.
    // They are passed from on_configure after lifecycle initialization has completed.
    pending_calibrations_ = std::move(calibrations);
  }
  catch (const std::exception & error)
  {
    RCLCPP_ERROR(get_logger(), "STS3215 initialization failed: %s", error.what());
    return CallbackReturn::ERROR;
  }
  return CallbackReturn::SUCCESS;
}

CallbackReturn STS3215System::on_configure(const rclcpp_lifecycle::State &)
{
  try
  {
    driver_->configure(port_, baud_rate_, pending_calibrations_);
  }
  catch (const std::exception & error)
  {
    RCLCPP_ERROR(get_logger(), "STS3215 configuration failed: %s", error.what());
    return CallbackReturn::ERROR;
  }
  return CallbackReturn::SUCCESS;
}

CallbackReturn STS3215System::on_activate(const rclcpp_lifecycle::State &)
{
  try
  {
    cache_interface_handles();
    publish_states(driver_->activate(), true);
  }
  catch (const std::exception & error)
  {
    RCLCPP_ERROR(get_logger(), "STS3215 activation failed: %s", error.what());
    return CallbackReturn::ERROR;
  }
  return CallbackReturn::SUCCESS;
}

CallbackReturn STS3215System::on_deactivate(const rclcpp_lifecycle::State &)
{
  try
  {
    driver_->deactivate();
  }
  catch (const std::exception & error)
  {
    RCLCPP_ERROR(get_logger(), "STS3215 deactivation failed: %s", error.what());
    return CallbackReturn::ERROR;
  }
  return CallbackReturn::SUCCESS;
}

CallbackReturn STS3215System::on_cleanup(const rclcpp_lifecycle::State &)
{
  driver_->close();
  return CallbackReturn::SUCCESS;
}

CallbackReturn STS3215System::on_error(const rclcpp_lifecycle::State &)
{
  driver_->close();
  return CallbackReturn::SUCCESS;
}

hardware_interface::return_type STS3215System::read(
  const rclcpp::Time &, const rclcpp::Duration &)
{
  try
  {
    publish_states(driver_->read(), false);
    return hardware_interface::return_type::OK;
  }
  catch (const std::exception & error)
  {
    RCLCPP_ERROR(get_logger(), "STS3215 read failed: %s", error.what());
    return hardware_interface::return_type::ERROR;
  }
}

hardware_interface::return_type STS3215System::write(
  const rclcpp::Time &, const rclcpp::Duration &)
{
  try
  {
    std::map<std::string, double> commands;
    const auto & calibrations = driver_->calibrations();
    for (std::size_t index = 0; index < calibrations.size(); ++index)
    {
      double value = 0.0;
      if (!get_command(position_command_handles_.at(index), value, false))
      {
        throw std::runtime_error("Could not read a position command interface.");
      }
      commands.emplace(calibrations[index].name, value);
    }
    driver_->write(commands);
    return hardware_interface::return_type::OK;
  }
  catch (const std::exception & error)
  {
    RCLCPP_ERROR(get_logger(), "STS3215 write failed: %s", error.what());
    return hardware_interface::return_type::ERROR;
  }
}

void STS3215System::cache_interface_handles()
{
  position_state_handles_.clear();
  velocity_state_handles_.clear();
  position_command_handles_.clear();
  for (const auto & joint : driver_->calibrations())
  {
    position_state_handles_.push_back(
      get_state_interface_handle(joint.name + "/" + hardware_interface::HW_IF_POSITION));
    velocity_state_handles_.push_back(
      get_state_interface_handle(joint.name + "/" + hardware_interface::HW_IF_VELOCITY));
    position_command_handles_.push_back(
      get_command_interface_handle(joint.name + "/" + hardware_interface::HW_IF_POSITION));
  }
}

void STS3215System::publish_states(
  const std::vector<JointState> & states, bool seed_commands)
{
  if (states.size() != position_state_handles_.size())
  {
    throw std::runtime_error("STS3215 state count does not match exported ROS interfaces.");
  }
  for (std::size_t index = 0; index < states.size(); ++index)
  {
    if (!set_state(position_state_handles_[index], states[index].position, false) ||
      !set_state(velocity_state_handles_[index], states[index].velocity, false))
    {
      throw std::runtime_error("Could not update an STS3215 state interface.");
    }
    if (seed_commands &&
      !set_command(position_command_handles_[index], states[index].position, false))
    {
      throw std::runtime_error("Could not seed an STS3215 position command interface.");
    }
  }
}

}  // namespace orion_hardware

PLUGINLIB_EXPORT_CLASS(orion_hardware::STS3215System, hardware_interface::SystemInterface)
