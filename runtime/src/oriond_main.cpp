#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <cerrno>
#include <chrono>
#include <csignal>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <iomanip>
#include <iostream>
#include <memory>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include "orion_hardware/ftservo_transport.hpp"
#include "orion_hardware/sts3215_driver.hpp"
#include "orion_runtime/joint_trajectory.hpp"
#include "orion_runtime/pose_library.hpp"
#include "orion_runtime/state_snapshot.hpp"

namespace
{

constexpr int kDefaultBaudRate = 1000000;
constexpr double kObserveFrequencyHz = 50.0;
constexpr auto kObservePeriod = std::chrono::milliseconds(20);
constexpr const char * kDefaultSocketPath = "/tmp/oriond.sock";

volatile std::sig_atomic_t g_stop_requested = 0;

const std::vector<std::string> kOrionJointNames = {
  "base_yaw_joint",
  "shoulder_pitch_joint",
  "elbow_pitch_joint",
  "head_roll_joint",
  "head_pitch_joint",
};

enum class Operation
{
  NONE,
  CHECK,
  SERVE,
  STATUS,
  CONFIGURE,
  ENABLE,
  DISABLE,
  GOTO,
};

struct Options
{
  Operation operation = Operation::NONE;
  bool help = false;
  std::string port = "/dev/ttyACM0";
  int baud_rate = kDefaultBaudRate;
  std::string calibration_file;
  std::string socket_path = kDefaultSocketPath;
  std::string poses_file = "ros2_ws/src/orion_motion/config/poses.yaml";
  std::string pose_name;
  double duration_seconds = 3.0;
};

class UnixStatusServer
{
public:
  explicit UnixStatusServer(std::string path)
  : path_(std::move(path))
  {
    if (path_.empty() || path_.size() >= sizeof(sockaddr_un::sun_path))
    {
      throw std::invalid_argument("Unix socket path is empty or too long.");
    }

    struct stat existing{};
    if (::lstat(path_.c_str(), &existing) == 0)
    {
      if (!S_ISSOCK(existing.st_mode))
      {
        throw std::runtime_error(
                "Refusing to replace non-socket path: " + path_);
      }
      if (::unlink(path_.c_str()) != 0)
      {
        throw_system_error("Could not remove stale Orion socket");
      }
    }
    else if (errno != ENOENT)
    {
      throw_system_error("Could not inspect Orion socket path");
    }

    fd_ = ::socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd_ < 0)
    {
      throw_system_error("Could not create Orion Unix socket");
    }

    sockaddr_un address{};
    address.sun_family = AF_UNIX;
    std::memcpy(address.sun_path, path_.c_str(), path_.size() + 1);
    if (::bind(fd_, reinterpret_cast<const sockaddr *>(&address), sizeof(address)) != 0)
    {
      const auto message = system_error_message("Could not bind Orion Unix socket");
      close_socket();
      throw std::runtime_error(message);
    }
    owns_path_ = true;
    if (::chmod(path_.c_str(), 0660) != 0)
    {
      const auto message = system_error_message("Could not set Orion socket permissions");
      close_socket();
      throw std::runtime_error(message);
    }
    if (::listen(fd_, 8) != 0)
    {
      const auto message = system_error_message("Could not listen on Orion Unix socket");
      close_socket();
      throw std::runtime_error(message);
    }
  }

  ~UnixStatusServer()
  {
    close_socket();
  }

  UnixStatusServer(const UnixStatusServer &) = delete;
  UnixStatusServer & operator=(const UnixStatusServer &) = delete;

  void serve_pending(const std::function<std::string(const std::string &)> & handler)
  {
    while (true)
    {
      const int client = ::accept4(fd_, nullptr, nullptr, SOCK_NONBLOCK | SOCK_CLOEXEC);
      if (client < 0)
      {
        if (errno == EAGAIN || errno == EWOULDBLOCK)
        {
          return;
        }
        throw_system_error("Could not accept Orion status client");
      }

      char request_buffer[256]{};
      const ssize_t received = ::recv(client, request_buffer, sizeof(request_buffer) - 1, 0);
      const std::string command = received > 0 ?
        trim(std::string(request_buffer, received)) : "";
      const std::string response = handler(command) + "\n";
      send_all(client, response);
      ::close(client);
    }
  }

private:
  static std::string trim(std::string value)
  {
    const auto first = value.find_first_not_of(" \t\r\n");
    if (first == std::string::npos)
    {
      return "";
    }
    const auto last = value.find_last_not_of(" \t\r\n");
    return value.substr(first, last - first + 1);
  }

  static std::string system_error_message(const std::string & operation)
  {
    return operation + ": " + std::strerror(errno);
  }

  [[noreturn]] static void throw_system_error(const std::string & operation)
  {
    throw std::runtime_error(system_error_message(operation));
  }

  static void send_all(int client, const std::string & response)
  {
    std::size_t sent = 0;
    while (sent < response.size())
    {
      const ssize_t count = ::send(
        client, response.data() + sent, response.size() - sent, MSG_NOSIGNAL);
      if (count > 0)
      {
        sent += static_cast<std::size_t>(count);
      }
      else if (count < 0 && errno == EINTR)
      {
        continue;
      }
      else
      {
        break;
      }
    }
  }

  void close_socket() noexcept
  {
    if (fd_ >= 0)
    {
      ::close(fd_);
      fd_ = -1;
    }
    if (owns_path_)
    {
      ::unlink(path_.c_str());
      owns_path_ = false;
    }
  }

  std::string path_;
  int fd_ = -1;
  bool owns_path_ = false;
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
    "Usage:\n"
    "  oriond --check  [--port DEVICE] [--baud-rate RATE] [--calibration FILE]\n"
    "  oriond --serve  [--port DEVICE] [--baud-rate RATE] [--calibration FILE] "
    "[--socket PATH]\n"
    "  oriond --status [--socket PATH]\n\n"
    "  oriond --configure [--socket PATH]\n"
    "  oriond --enable    [--socket PATH]\n"
    "  oriond --disable   [--socket PATH]\n\n"
    "  oriond --goto POSE [--duration SECONDS] [--socket PATH]\n\n"
    "  --check             Print one direct hardware state snapshot and exit.\n"
    "  --serve             Sample hardware at 50 Hz and serve local status JSON.\n"
    "  --status            Request the latest JSON snapshot from the daemon.\n"
    "  --configure         Apply and verify Orion's servo profile, torque off.\n"
    "  --enable            Seed measured positions, then enable holding torque.\n"
    "  --disable           Disable holding torque.\n"
    "  --goto POSE         Move all five joints to a named Orion pose.\n"
    "  --duration SECONDS  Quintic move duration (default: 3.0).\n"
    "  --port DEVICE       Servo serial device (default: /dev/ttyACM0).\n"
    "  --baud-rate RATE     Servo bus rate (default: 1000000).\n"
    "  --calibration FILE  Orion calibration JSON file.\n"
    "  --socket PATH       Local API socket (default: /tmp/oriond.sock).\n"
    "  --poses FILE        Pose library used by --serve.\n"
    "  --help               Show this help.\n\n"
    "Check and serve startup never enable torque or write servo registers.\n";
}

std::string require_value(int argc, char ** argv, int & index, const std::string & option)
{
  if (index + 1 >= argc)
  {
    throw std::invalid_argument(option + " requires a value.");
  }
  return argv[++index];
}

void select_operation(Options & options, Operation operation, const std::string & argument)
{
  if (options.operation != Operation::NONE)
  {
    throw std::invalid_argument("Select exactly one operation; repeated at " + argument + ".");
  }
  options.operation = operation;
}

Options parse_options(int argc, char ** argv)
{
  Options options;
  for (int index = 1; index < argc; ++index)
  {
    const std::string argument = argv[index];
    if (argument == "--check")
    {
      select_operation(options, Operation::CHECK, argument);
    }
    else if (argument == "--serve")
    {
      select_operation(options, Operation::SERVE, argument);
    }
    else if (argument == "--status")
    {
      select_operation(options, Operation::STATUS, argument);
    }
    else if (argument == "--configure")
    {
      select_operation(options, Operation::CONFIGURE, argument);
    }
    else if (argument == "--enable")
    {
      select_operation(options, Operation::ENABLE, argument);
    }
    else if (argument == "--disable")
    {
      select_operation(options, Operation::DISABLE, argument);
    }
    else if (argument == "--goto")
    {
      select_operation(options, Operation::GOTO, argument);
      options.pose_name = require_value(argc, argv, index, argument);
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
    else if (argument == "--socket")
    {
      options.socket_path = require_value(argc, argv, index, argument);
    }
    else if (argument == "--poses")
    {
      options.poses_file = require_value(argc, argv, index, argument);
    }
    else if (argument == "--duration")
    {
      options.duration_seconds = std::stod(require_value(argc, argv, index, argument));
    }
    else
    {
      throw std::invalid_argument("Unknown option: " + argument);
    }
  }

  if (!options.help &&
    (options.operation == Operation::CHECK || options.operation == Operation::SERVE) &&
    options.calibration_file.empty())
  {
    options.calibration_file = default_calibration_file();
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

std::unique_ptr<orion_hardware::Sts3215Driver> connect_driver(const Options & options)
{
  auto transport = std::make_shared<orion_hardware::FtServoTransport>();
  auto driver = std::make_unique<orion_hardware::Sts3215Driver>(transport);
  const auto calibrations = orion_hardware::load_calibration_file(
    options.calibration_file, kOrionJointNames);
  driver->connect(options.port, options.baud_rate, calibrations);
  return driver;
}

void request_stop(int)
{
  g_stop_requested = 1;
}

int serve(const Options & options)
{
  auto driver = connect_driver(options);
  const orion_runtime::PoseLibrary pose_library(options.poses_file, kOrionJointNames);
  orion_runtime::RuntimeMode mode = orion_runtime::RuntimeMode::OBSERVE;
  std::optional<orion_runtime::JointTrajectory> trajectory;
  std::chrono::steady_clock::time_point trajectory_started_at;
  std::uint64_t sequence = 1;
  auto snapshot = orion_runtime::make_state_snapshot(
    mode, sequence, kObserveFrequencyHz, driver->read());
  UnixStatusServer server(options.socket_path);

  std::signal(SIGINT, request_stop);
  std::signal(SIGTERM, request_stop);
  std::cout << "oriond: observing at 50 Hz on " << options.socket_path << '\n';

  auto next_sample = std::chrono::steady_clock::now();
  while (g_stop_requested == 0)
  {
    next_sample += kObservePeriod;
    const auto states = driver->read();
    std::string active_motion;
    double motion_progress = 0.0;
    if (trajectory.has_value())
    {
      const double elapsed = std::chrono::duration<double>(
        std::chrono::steady_clock::now() - trajectory_started_at).count();
      driver->write(trajectory->sample(elapsed));
      active_motion = trajectory->name();
      motion_progress = trajectory->progress(elapsed);
      if (trajectory->complete(elapsed))
      {
        trajectory.reset();
        mode = orion_runtime::RuntimeMode::HOLDING;
        active_motion.clear();
        motion_progress = 0.0;
      }
    }
    snapshot = orion_runtime::make_state_snapshot(
      mode, ++sequence, kObserveFrequencyHz, states, active_motion, motion_progress);
    server.serve_pending(
      [&](const std::string & command) -> std::string {
        if (command == "status")
        {
          return orion_runtime::state_snapshot_to_json(snapshot);
        }
        if (command == "configure")
        {
          if (orion_runtime::torque_is_enabled(mode))
          {
            return "{\"ok\":false,\"command\":\"configure\","
              "\"error\":\"disable torque before configuring\"}";
          }
          driver->apply_servo_profile();
          mode = orion_runtime::RuntimeMode::CONFIGURED;
          snapshot = orion_runtime::make_state_snapshot(
            mode, ++sequence, kObserveFrequencyHz, driver->read());
          return "{\"ok\":true,\"command\":\"configure\",\"mode\":\"configured\"}";
        }
        if (command == "enable")
        {
          if (mode != orion_runtime::RuntimeMode::CONFIGURED)
          {
            return "{\"ok\":false,\"command\":\"enable\","
              "\"error\":\"configure before enabling torque\"}";
          }
          snapshot = orion_runtime::make_state_snapshot(
            orion_runtime::RuntimeMode::HOLDING, ++sequence,
            kObserveFrequencyHz, driver->activate());
          mode = orion_runtime::RuntimeMode::HOLDING;
          return "{\"ok\":true,\"command\":\"enable\",\"mode\":\"holding\"}";
        }
        if (command == "disable")
        {
          if (!orion_runtime::torque_is_enabled(mode))
          {
            return "{\"ok\":false,\"command\":\"disable\","
              "\"error\":\"torque is not enabled\"}";
          }
          trajectory.reset();
          driver->deactivate();
          mode = orion_runtime::RuntimeMode::CONFIGURED;
          snapshot = orion_runtime::make_state_snapshot(
            mode, ++sequence, kObserveFrequencyHz, driver->read());
          return "{\"ok\":true,\"command\":\"disable\",\"mode\":\"configured\"}";
        }
        if (command.rfind("goto ", 0) == 0)
        {
          if (mode != orion_runtime::RuntimeMode::HOLDING)
          {
            return "{\"ok\":false,\"command\":\"goto\","
              "\"error\":\"enable holding torque before moving\"}";
          }
          std::istringstream request(command);
          std::string verb;
          std::string pose_name;
          double duration_seconds = 0.0;
          std::string trailing;
          if (!(request >> verb >> pose_name >> duration_seconds) || request >> trailing)
          {
            return "{\"ok\":false,\"command\":\"goto\","
              "\"error\":\"expected goto POSE SECONDS\"}";
          }

          const auto & target = pose_library.pose(pose_name);
          driver->validate_positions(target);
          orion_runtime::JointPositions start;
          for (const auto & joint : snapshot.joints)
          {
            start.emplace(joint.name, joint.position);
          }
          trajectory.emplace(
            pose_name, driver->clamp_positions_to_safe_range(start), target,
            duration_seconds);
          trajectory_started_at = std::chrono::steady_clock::now();
          mode = orion_runtime::RuntimeMode::MOVING;
          return "{\"ok\":true,\"command\":\"goto\",\"pose\":\"" +
            pose_name + "\",\"mode\":\"moving\",\"duration_seconds\":" +
            std::to_string(duration_seconds) + "}";
        }
        return "{\"ok\":false,\"error\":\"unknown Orion daemon command\"}";
      });
    std::this_thread::sleep_until(next_sample);
  }
  return 0;
}

int request_daemon(const std::string & socket_path, const std::string & command)
{
  if (socket_path.empty() || socket_path.size() >= sizeof(sockaddr_un::sun_path))
  {
    throw std::invalid_argument("Unix socket path is empty or too long.");
  }
  const int client = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
  if (client < 0)
  {
    throw std::runtime_error("Could not create Orion status client: " +
            std::string(std::strerror(errno)));
  }

  sockaddr_un address{};
  address.sun_family = AF_UNIX;
  std::memcpy(address.sun_path, socket_path.c_str(), socket_path.size() + 1);
  if (::connect(client, reinterpret_cast<const sockaddr *>(&address), sizeof(address)) != 0)
  {
    const std::string message = "Could not connect to Orion daemon: " +
      std::string(std::strerror(errno));
    ::close(client);
    throw std::runtime_error(message);
  }
  const std::string request = command + "\n";
  if (::send(client, request.data(), request.size(), MSG_NOSIGNAL) !=
    static_cast<ssize_t>(request.size()))
  {
    const std::string message = "Could not request Orion status: " +
      std::string(std::strerror(errno));
    ::close(client);
    throw std::runtime_error(message);
  }

  std::string response;
  char buffer[1024];
  while (true)
  {
    const ssize_t count = ::recv(client, buffer, sizeof(buffer), 0);
    if (count > 0)
    {
      response.append(buffer, static_cast<std::size_t>(count));
    }
    else if (count == 0)
    {
      break;
    }
    else if (errno != EINTR)
    {
      const std::string message = "Could not read Orion status: " +
        std::string(std::strerror(errno));
      ::close(client);
      throw std::runtime_error(message);
    }
  }
  ::close(client);
  std::cout << response;
  return 0;
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

    switch (options.operation)
    {
      case Operation::CHECK:
      {
        auto driver = connect_driver(options);
        print_states(driver->read());
        return 0;
      }
      case Operation::SERVE:
        return serve(options);
      case Operation::STATUS:
        return request_daemon(options.socket_path, "status");
      case Operation::CONFIGURE:
        return request_daemon(options.socket_path, "configure");
      case Operation::ENABLE:
        return request_daemon(options.socket_path, "enable");
      case Operation::DISABLE:
        return request_daemon(options.socket_path, "disable");
      case Operation::GOTO:
        return request_daemon(
          options.socket_path,
          "goto " + options.pose_name + " " + std::to_string(options.duration_seconds));
      case Operation::NONE:
        print_usage(std::cerr);
        return 2;
    }
  }
  catch (const std::exception & error)
  {
    std::cerr << "oriond: " << error.what() << '\n';
    return 1;
  }
  return 1;
}
