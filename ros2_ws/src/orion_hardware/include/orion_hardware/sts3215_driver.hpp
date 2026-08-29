#ifndef ORION_HARDWARE__STS3215_DRIVER_HPP_
#define ORION_HARDWARE__STS3215_DRIVER_HPP_

#include <cstdint>
#include <map>
#include <memory>
#include <string>
#include <vector>

#include "orion_hardware/sts3215_transport.hpp"

namespace orion_hardware
{

inline constexpr int kSts3215ModelNumber = 777;
inline constexpr int kEncoderResolution = 4096;
inline constexpr double kVelocityRawToRadiansPerSecond =
  0.732 * 2.0 * 3.14159265358979323846 / 60.0;

struct JointCalibration
{
  std::string name;
  std::uint8_t servo_id = 0;
  int neutral_raw = 0;
  int encoder_direction = 1;
  int safe_min_delta_raw = 0;
  int safe_max_delta_raw = 0;
};

struct JointState
{
  std::string name;
  double position = 0.0;
  double velocity = 0.0;
  double current_ma = 0.0;
  double voltage_v = 0.0;
  double temperature_c = 0.0;
  int status = 0;
};

struct JointServoProfile
{
  int return_delay_time = 0;
  int operating_mode = 0;
  int drive_mode = 0;
  int p_coefficient = 16;
  int i_coefficient = 0;
  int d_coefficient = 32;
  int maximum_acceleration = 254;
  int acceleration = 254;
};

using ServoProfiles = std::map<std::string, JointServoProfile>;

ServoProfiles make_orion_servo_profiles();

class Sts3215Driver
{
public:
  explicit Sts3215Driver(
    std::shared_ptr<Sts3215Transport> transport,
    ServoProfiles servo_profiles = make_orion_servo_profiles());
  ~Sts3215Driver();

  void connect(
    const std::string & port, int baud_rate,
    const std::vector<JointCalibration> & calibrations);
  void apply_servo_profile();
  void configure(
    const std::string & port, int baud_rate,
    const std::vector<JointCalibration> & calibrations);
  std::vector<JointState> activate();
  void deactivate();
  void close() noexcept;

  std::vector<JointState> read();
  std::map<std::string, double> clamp_positions_to_safe_range(
    const std::map<std::string, double> & positions_radians) const;
  void validate_positions(const std::map<std::string, double> & positions_radians) const;
  void write(const std::map<std::string, double> & positions_radians);

  bool is_active() const noexcept;
  const std::vector<JointCalibration> & calibrations() const noexcept;

private:
  std::vector<JointState> convert_states(
    const std::map<std::uint8_t, Sts3215RawState> & raw_states) const;
  std::map<std::uint8_t, int> encode_positions(
    const std::map<std::string, double> & positions_radians) const;
  int radians_to_raw(const JointCalibration & joint, double radians) const;
  double raw_to_radians(const JointCalibration & joint, int raw_position) const;

  std::shared_ptr<Sts3215Transport> transport_;
  ServoProfiles servo_profiles_;
  std::vector<JointCalibration> calibrations_;
  bool configured_ = false;
  bool profile_applied_ = false;
  bool active_ = false;
};

std::vector<JointCalibration> load_calibration_file(
  const std::string & path, const std::vector<std::string> & expected_joint_names);

}  // namespace orion_hardware

#endif  // ORION_HARDWARE__STS3215_DRIVER_HPP_
