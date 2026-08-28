#ifndef ORION_HARDWARE__STS3215_TRANSPORT_HPP_
#define ORION_HARDWARE__STS3215_TRANSPORT_HPP_

#include <cstdint>
#include <map>
#include <string>
#include <vector>

namespace orion_hardware
{

enum class Sts3215Register
{
  MODEL_NUMBER,
  FIRMWARE_MAJOR_VERSION,
  FIRMWARE_MINOR_VERSION,
  RETURN_DELAY_TIME,
  MAX_TORQUE_LIMIT,
  PHASE,
  P_COEFFICIENT,
  D_COEFFICIENT,
  I_COEFFICIENT,
  PROTECTION_CURRENT,
  OPERATING_MODE,
  TORQUE_ENABLE,
  ACCELERATION,
  GOAL_POSITION,
  GOAL_VELOCITY,
  TORQUE_LIMIT,
  PRESENT_POSITION,
  PRESENT_VELOCITY,
  PRESENT_CURRENT,
  PRESENT_VOLTAGE,
  PRESENT_TEMPERATURE,
  STATUS,
  MAXIMUM_VELOCITY_LIMIT,
  MAXIMUM_ACCELERATION,
};

struct Sts3215RawState
{
  int position = 0;
  int velocity = 0;
  int current = 0;
  int voltage = 0;
  int temperature = 0;
  int status = 0;
};

class Sts3215Transport
{
public:
  virtual ~Sts3215Transport() = default;

  virtual void open(const std::string & port, int baud_rate) = 0;
  virtual void close() noexcept = 0;
  virtual bool is_open() const noexcept = 0;

  virtual int read_register(std::uint8_t servo_id, Sts3215Register register_name) = 0;
  virtual void write_register(
    std::uint8_t servo_id, Sts3215Register register_name, int value) = 0;
  virtual void set_eeprom_lock(
    const std::vector<std::uint8_t> & servo_ids, bool locked) = 0;
  virtual std::map<std::uint8_t, Sts3215RawState> read_states(
    const std::vector<std::uint8_t> & servo_ids) = 0;
  virtual void write_positions(const std::map<std::uint8_t, int> & positions) = 0;
  virtual void set_torque(
    const std::vector<std::uint8_t> & servo_ids, bool enabled) = 0;
};

}  // namespace orion_hardware

#endif  // ORION_HARDWARE__STS3215_TRANSPORT_HPP_
