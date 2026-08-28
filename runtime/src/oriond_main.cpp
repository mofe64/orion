#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include "orion_hardware/ftservo_transport.hpp"
#include "orion_hardware/sts3215_driver.hpp"

namespace
{

constexpr int kDefaultBaudRate = 1000000;

const std::vector<std::string> kOrionJointNames = {
  "base_yaw_joint",
  "shoulder_pitch_joint",
  "elbow_pitch_joint",
  "head_roll_joint",
  "head_pitch_joint",
};

struct Options
{
  bool check = false;
  bool help = false;
  std::string port = "/dev/ttyACM0";
  int baud_rate = kDefaultBaudRate;
  std::string calibration_file;
};

std::string default_calibration_file()
{
  const char * home = std::getenv("HOME");
  if (home == nullptr || std::string(home).empty())
  {
    throw std::runtime_error(
            "HOME is not set; pass --calibration with an absolute path.");
  }
  return std::string(home) + "/.config/orion/servo_calibration.json";
}

void print_usage(std::ostream & output)
{
  output <<
    "Usage: oriond --check [--port DEVICE] [--baud-rate RATE] "
    "[--calibration FILE]\n\n"
    "  --check             Validate the five-servo bus and print one state snapshot.\n"
    "                      This mode never enables torque or writes servo registers.\n"
    "  --port DEVICE       Servo serial device (default: /dev/ttyACM0).\n"
    "  --baud-rate RATE     Servo bus rate (default: 1000000).\n"
    "  --calibration FILE  Orion calibration JSON file.\n"
    "  --help               Show this help.\n";
}

std::string require_value(int argc, char ** argv, int & index, const std::string & option)
{
  if (index + 1 >= argc)
  {
    throw std::invalid_argument(option + " requires a value.");
  }
  return argv[++index];
}

Options parse_options(int argc, char ** argv)
{
  Options options;
  options.calibration_file = default_calibration_file();
  for (int index = 1; index < argc; ++index)
  {
    const std::string argument = argv[index];
    if (argument == "--check")
    {
      options.check = true;
    }
    else if (argument == "--help" || argument == "-h")
    {
      options.help = true;
    }
    else if (argument == "--port")
    {
      options.port = require_value(argc, argv, index, argument);
    }
    else if (argument == "--baud-rate")
    {
      options.baud_rate = std::stoi(require_value(argc, argv, index, argument));
    }
    else if (argument == "--calibration")
    {
      options.calibration_file = require_value(argc, argv, index, argument);
    }
    else
    {
      throw std::invalid_argument("Unknown option: " + argument);
    }
  }
  return options;
}

void print_states(const std::vector<orion_hardware::JointState> & states)
{
  std::cout << std::left
            << std::setw(25) << "joint"
            << std::right
            << std::setw(13) << "position"
            << std::setw(13) << "velocity"
            << std::setw(12) << "current"
            << std::setw(10) << "voltage"
            << std::setw(8) << "temp"
            << std::setw(8) << "status" << '\n';
  std::cout << std::fixed << std::setprecision(3);
  for (const auto & state : states)
  {
    std::cout << std::left << std::setw(25) << state.name
              << std::right
              << std::setw(13) << state.position
              << std::setw(13) << state.velocity
              << std::setw(12) << state.current_ma
              << std::setw(10) << state.voltage_v
              << std::setw(8) << state.temperature_c
              << std::setw(8) << state.status << '\n';
  }
}

}  // namespace

int main(int argc, char ** argv)
{
  try
  {
    const auto options = parse_options(argc, argv);
    if (options.help)
    {
      print_usage(std::cout);
      return 0;
    }
    if (!options.check)
    {
      print_usage(std::cerr);
      return 2;
    }

    auto transport = std::make_shared<orion_hardware::FtServoTransport>();
    orion_hardware::Sts3215Driver driver(transport);
    const auto calibrations = orion_hardware::load_calibration_file(
      options.calibration_file, kOrionJointNames);
    driver.connect(options.port, options.baud_rate, calibrations);
    print_states(driver.read());
    return 0;
  }
  catch (const std::exception & error)
  {
    std::cerr << "oriond: " << error.what() << '\n';
    return 1;
  }
}
