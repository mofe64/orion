#include "orion_hardware/sts3215_driver.hpp"

#include <algorithm>
#include <cmath>
#include <set>
#include <stdexcept>
#include <tuple>
#include <utility>

namespace orion_hardware
{
namespace
{

using PendingWrite = std::tuple<std::uint8_t, Sts3215Register, int>;

int wrap_raw(int value)
{
  const int remainder = value % kEncoderResolution;
  return remainder < 0 ? remainder + kEncoderResolution : remainder;
}

int circular_delta(int raw, int neutral)
{
  return wrap_raw(raw - neutral + kEncoderResolution / 2) - kEncoderResolution / 2;
}

std::vector<std::uint8_t> servo_ids(const std::vector<JointCalibration> & calibrations)
{
  std::vector<std::uint8_t> ids;
  ids.reserve(calibrations.size());
  for (const auto & joint : calibrations)
  {
    ids.push_back(joint.servo_id);
  }
  return ids;
}

}  // namespace

ServoProfiles make_orion_servo_profiles()
{
  ServoProfiles profiles;
  for (const auto * joint_name : {
      "base_yaw_joint",
      "shoulder_pitch_joint",
      "elbow_pitch_joint",
      "head_roll_joint",
      "head_pitch_joint"})
  {
    profiles.emplace(joint_name, JointServoProfile{});
  }

  // The elbow showed a 51-count holding error at P=16 under the assembled head load.
  // Restore its factory P=32 as the first measured tuning step; leave every other
  // profile value unchanged so the physical trial isolates this one variable.
  profiles.at("elbow_pitch_joint").p_coefficient = 32;
  return profiles;
}

Sts3215Driver::Sts3215Driver(
  std::shared_ptr<Sts3215Transport> transport, ServoProfiles servo_profiles)
: transport_(std::move(transport)), servo_profiles_(std::move(servo_profiles))
{
  if (!transport_)
  {
    throw std::invalid_argument("STS3215 transport must not be null.");
  }
}

Sts3215Driver::~Sts3215Driver()
{
  close();
}

void Sts3215Driver::configure(
  const std::string & port, int baud_rate,
  const std::vector<JointCalibration> & calibrations)
{
  connect(port, baud_rate, calibrations);
  apply_servo_profile();
}

void Sts3215Driver::connect(
  const std::string & port, int baud_rate,
  const std::vector<JointCalibration> & calibrations)
{
  if (port.empty() || baud_rate <= 0 || calibrations.empty())
  {
    throw std::invalid_argument("Port, baud rate, and calibrations are required.");
  }

  std::set<std::string> names;
  std::set<std::uint8_t> ids;
  for (const auto & joint : calibrations)
  {
    if (!names.insert(joint.name).second || !ids.insert(joint.servo_id).second)
    {
      throw std::invalid_argument("Joint names and servo IDs must be unique.");
    }
  }

  close();
  transport_->open(port, baud_rate);
  try
  {
    int firmware_major = -1;
    int firmware_minor = -1;

    for (const auto & joint : calibrations)
    {
      const auto id = joint.servo_id;
      if (transport_->read_register(id, Sts3215Register::MODEL_NUMBER) !=
        kSts3215ModelNumber)
      {
        throw std::runtime_error("Servo " + std::to_string(id) + " is not an STS3215.");
      }
      if (transport_->read_register(id, Sts3215Register::TORQUE_ENABLE) != 0)
      {
        throw std::runtime_error(
                "Servo " + std::to_string(id) + " must have torque off during configuration.");
      }
      if (transport_->read_register(id, Sts3215Register::STATUS) != 0)
      {
        throw std::runtime_error(
                "Servo " + std::to_string(id) + " reported a fault before configuration.");
      }

      const int major =
        transport_->read_register(id, Sts3215Register::FIRMWARE_MAJOR_VERSION);
      const int minor =
        transport_->read_register(id, Sts3215Register::FIRMWARE_MINOR_VERSION);
      if (firmware_major < 0)
      {
        firmware_major = major;
        firmware_minor = minor;
      }
      else if (major != firmware_major || minor != firmware_minor)
      {
        throw std::runtime_error("All five STS3215 servos must use the same firmware.");
      }
    }

    calibrations_ = calibrations;
    configured_ = true;
    profile_applied_ = false;
    active_ = false;
  }
  catch (...)
  {
    configured_ = false;
    profile_applied_ = false;
    active_ = false;
    transport_->close();
    throw;
  }
}

void Sts3215Driver::apply_servo_profile()
{
  if (!configured_ || active_)
  {
    throw std::logic_error(
            "STS3215 driver must be connected and inactive before applying its profile.");
  }

  try
  {
    const auto all_ids = servo_ids(calibrations_);
    std::vector<PendingWrite> persistent_writes;
    for (const auto & joint : calibrations_)
    {
      const auto id = joint.servo_id;
      const auto profile_found = servo_profiles_.find(joint.name);
      if (profile_found == servo_profiles_.end())
      {
        throw std::runtime_error("Missing STS3215 servo profile for " + joint.name + ".");
      }
      const auto & profile = profile_found->second;
      if (profile.drive_mode != 0 && profile.drive_mode != 1)
      {
        throw std::runtime_error("STS3215 drive mode must be 0 or 1 for " + joint.name + ".");
      }
      const auto queue_if_different =
        [this, id, &persistent_writes](Sts3215Register register_name, int desired) {
          if (transport_->read_register(id, register_name) != desired)
          {
            persistent_writes.emplace_back(id, register_name, desired);
          }
        };

      queue_if_different(Sts3215Register::RETURN_DELAY_TIME, profile.return_delay_time);
      queue_if_different(Sts3215Register::OPERATING_MODE, profile.operating_mode);
      queue_if_different(Sts3215Register::P_COEFFICIENT, profile.p_coefficient);
      queue_if_different(Sts3215Register::I_COEFFICIENT, profile.i_coefficient);
      queue_if_different(Sts3215Register::D_COEFFICIENT, profile.d_coefficient);
      queue_if_different(
        Sts3215Register::MAXIMUM_ACCELERATION, profile.maximum_acceleration);

      const int phase = transport_->read_register(id, Sts3215Register::PHASE);
      const int desired_phase = profile.drive_mode == 0 ? phase & ~0x10 : phase | 0x10;
      if (phase != desired_phase)
      {
        persistent_writes.emplace_back(id, Sts3215Register::PHASE, desired_phase);
      }
    }

    if (!persistent_writes.empty())
    {
      try
      {
        transport_->set_eeprom_lock(all_ids, false);
        for (const auto & [id, register_name, value] : persistent_writes)
        {
          transport_->write_register(id, register_name, value);
          if (transport_->read_register(id, register_name) != value)
          {
            throw std::runtime_error("STS3215 persistent configuration verification failed.");
          }
        }
      }
      catch (...)
      {
        try
        {
          transport_->set_eeprom_lock(all_ids, true);
        }
        catch (...)
        {
        }
        throw;
      }
      transport_->set_eeprom_lock(all_ids, true);
    }

    for (const auto & joint : calibrations_)
    {
      const auto id = joint.servo_id;
      const int acceleration = servo_profiles_.at(joint.name).acceleration;
      if (transport_->read_register(id, Sts3215Register::ACCELERATION) != acceleration)
      {
        transport_->write_register(id, Sts3215Register::ACCELERATION, acceleration);
      }
      if (transport_->read_register(id, Sts3215Register::ACCELERATION) != acceleration)
      {
        throw std::runtime_error("STS3215 runtime acceleration verification failed.");
      }
    }
    profile_applied_ = true;
  }
  catch (...)
  {
    close();
    throw;
  }
}

std::vector<JointState> Sts3215Driver::activate()
{
  if (!configured_ || !profile_applied_ || active_)
  {
    throw std::logic_error(
            "STS3215 driver must have its profile applied and be inactive before activation.");
  }

  const auto ids = servo_ids(calibrations_);
  const auto raw_states = transport_->read_states(ids);
  std::map<std::uint8_t, int> initial_goals;
  for (const auto & joint : calibrations_)
  {
    const auto found = raw_states.find(joint.servo_id);
    if (found == raw_states.end())
    {
      throw std::runtime_error("Missing STS3215 state during activation.");
    }
    if (found->second.status != 0)
    {
      throw std::runtime_error("STS3215 reported a fault during activation.");
    }
    initial_goals.emplace(joint.servo_id, found->second.position);
  }

  transport_->write_positions(initial_goals);
  for (const auto & [id, expected_goal] : initial_goals)
  {
    if (transport_->read_register(id, Sts3215Register::GOAL_POSITION) != expected_goal)
    {
      throw std::runtime_error(
              "STS3215 initial goal verification failed for servo " + std::to_string(id) + ".");
    }
  }
  try
  {
    transport_->set_torque(ids, true);
  }
  catch (...)
  {
    try
    {
      transport_->set_torque(ids, false);
    }
    catch (...)
    {
    }
    throw;
  }
  active_ = true;
  return convert_states(raw_states);
}

void Sts3215Driver::deactivate()
{
  if (configured_ && transport_->is_open())
  {
    transport_->set_torque(servo_ids(calibrations_), false);
  }
  active_ = false;
}

void Sts3215Driver::close() noexcept
{
  if (!transport_->is_open())
  {
    configured_ = false;
    profile_applied_ = false;
    active_ = false;
    return;
  }
  if (active_)
  {
    try
    {
      transport_->set_torque(servo_ids(calibrations_), false);
    }
    catch (...)
    {
    }
  }
  transport_->close();
  configured_ = false;
  profile_applied_ = false;
  active_ = false;
}

std::vector<JointState> Sts3215Driver::read()
{
  if (!configured_)
  {
    throw std::logic_error("STS3215 driver is not configured.");
  }
  return convert_states(transport_->read_states(servo_ids(calibrations_)));
}

void Sts3215Driver::write(const std::map<std::string, double> & positions_radians)
{
  if (!active_)
  {
    throw std::logic_error("STS3215 driver is not active.");
  }
  transport_->write_positions(encode_positions(positions_radians));
}

std::map<std::string, double> Sts3215Driver::clamp_positions_to_safe_range(
  const std::map<std::string, double> & positions_radians) const
{
  if (!configured_)
  {
    throw std::logic_error("STS3215 driver is not configured.");
  }
  if (positions_radians.size() != calibrations_.size())
  {
    throw std::invalid_argument("A position is required for every Orion joint.");
  }

  const double radians_per_step =
    2.0 * 3.14159265358979323846 / kEncoderResolution;
  std::map<std::string, double> clamped;
  for (const auto & joint : calibrations_)
  {
    const auto position = positions_radians.find(joint.name);
    if (position == positions_radians.end())
    {
      throw std::invalid_argument("Missing position for " + joint.name + ".");
    }
    if (!std::isfinite(position->second))
    {
      throw std::invalid_argument(joint.name + " position must be finite.");
    }

    const double first_limit =
      joint.safe_min_delta_raw * radians_per_step / joint.encoder_direction;
    const double second_limit =
      joint.safe_max_delta_raw * radians_per_step / joint.encoder_direction;
    const double minimum = std::min(first_limit, second_limit);
    const double maximum = std::max(first_limit, second_limit);
    clamped.emplace(joint.name, std::clamp(position->second, minimum, maximum));
  }
  return clamped;
}

void Sts3215Driver::validate_positions(
  const std::map<std::string, double> & positions_radians) const
{
  if (!configured_)
  {
    throw std::logic_error("STS3215 driver is not configured.");
  }
  (void)encode_positions(positions_radians);
}

std::map<std::uint8_t, int> Sts3215Driver::encode_positions(
  const std::map<std::string, double> & positions_radians) const
{
  if (positions_radians.size() != calibrations_.size())
  {
    throw std::invalid_argument("A position command is required for every Orion joint.");
  }

  std::map<std::uint8_t, int> raw_positions;
  for (const auto & joint : calibrations_)
  {
    const auto command = positions_radians.find(joint.name);
    if (command == positions_radians.end())
    {
      throw std::invalid_argument("Missing position command for " + joint.name + ".");
    }
    raw_positions.emplace(joint.servo_id, radians_to_raw(joint, command->second));
  }
  return raw_positions;
}

bool Sts3215Driver::is_active() const noexcept
{
  return active_;
}

const std::vector<JointCalibration> & Sts3215Driver::calibrations() const noexcept
{
  return calibrations_;
}

std::vector<JointState> Sts3215Driver::convert_states(
  const std::map<std::uint8_t, Sts3215RawState> & raw_states) const
{
  std::vector<JointState> result;
  result.reserve(calibrations_.size());
  for (const auto & joint : calibrations_)
  {
    const auto found = raw_states.find(joint.servo_id);
    if (found == raw_states.end())
    {
      throw std::runtime_error("Missing STS3215 state for " + joint.name + ".");
    }
    const auto & raw = found->second;
    result.push_back(JointState{
      joint.name,
      raw_to_radians(joint, raw.position),
      raw.velocity * kVelocityRawToRadiansPerSecond * joint.encoder_direction,
      raw.current * 6.5,
      raw.voltage / 10.0,
      static_cast<double>(raw.temperature),
      raw.status,
    });
  }
  return result;
}

int Sts3215Driver::radians_to_raw(const JointCalibration & joint, double radians) const
{
  if (!std::isfinite(radians))
  {
    throw std::invalid_argument(joint.name + " position command must be finite.");
  }
  const double steps_per_radian =
    kEncoderResolution / (2.0 * 3.14159265358979323846);
  const int delta =
    static_cast<int>(std::lround(radians * steps_per_radian)) * joint.encoder_direction;
  if (delta < joint.safe_min_delta_raw || delta > joint.safe_max_delta_raw)
  {
    throw std::out_of_range(joint.name + " command is outside its calibrated safe range.");
  }
  return wrap_raw(joint.neutral_raw + delta);
}

double Sts3215Driver::raw_to_radians(
  const JointCalibration & joint, int raw_position) const
{
  if (raw_position < 0 || raw_position >= kEncoderResolution)
  {
    throw std::runtime_error(joint.name + " returned an invalid raw encoder position.");
  }
  const int delta = circular_delta(raw_position, joint.neutral_raw);
  const double steps_per_radian =
    kEncoderResolution / (2.0 * 3.14159265358979323846);
  return delta / (steps_per_radian * joint.encoder_direction);
}

}  // namespace orion_hardware
