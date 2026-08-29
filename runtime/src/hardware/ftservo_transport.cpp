#include "orion_hardware/ftservo_transport.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "SMS_STS.h"

namespace orion_hardware
{
namespace
{

struct RegisterInfo
{
  std::uint8_t address;
  std::uint8_t width;
  bool writable;
};

RegisterInfo register_info(Sts3215Register register_name)
{
  switch (register_name)
  {
    case Sts3215Register::FIRMWARE_MAJOR_VERSION:
      return {0, 1, false};
    case Sts3215Register::FIRMWARE_MINOR_VERSION:
      return {1, 1, false};
    case Sts3215Register::MODEL_NUMBER:
      return {3, 2, false};
    case Sts3215Register::RETURN_DELAY_TIME:
      return {7, 1, true};
    case Sts3215Register::MAX_TORQUE_LIMIT:
      return {16, 2, true};
    case Sts3215Register::PHASE:
      return {18, 1, true};
    case Sts3215Register::P_COEFFICIENT:
      return {21, 1, true};
    case Sts3215Register::D_COEFFICIENT:
      return {22, 1, true};
    case Sts3215Register::I_COEFFICIENT:
      return {23, 1, true};
    case Sts3215Register::PROTECTION_CURRENT:
      return {28, 2, true};
    case Sts3215Register::OPERATING_MODE:
      return {33, 1, true};
    case Sts3215Register::TORQUE_ENABLE:
      return {40, 1, true};
    case Sts3215Register::ACCELERATION:
      return {41, 1, true};
    case Sts3215Register::GOAL_POSITION:
      return {42, 2, true};
    case Sts3215Register::GOAL_VELOCITY:
      return {46, 2, true};
    case Sts3215Register::TORQUE_LIMIT:
      return {48, 2, true};
    case Sts3215Register::PRESENT_POSITION:
      return {56, 2, false};
    case Sts3215Register::PRESENT_VELOCITY:
      return {58, 2, false};
    case Sts3215Register::PRESENT_VOLTAGE:
      return {62, 1, false};
    case Sts3215Register::PRESENT_TEMPERATURE:
      return {63, 1, false};
    case Sts3215Register::STATUS:
      return {65, 1, false};
    case Sts3215Register::PRESENT_CURRENT:
      return {69, 2, false};
    case Sts3215Register::MAXIMUM_VELOCITY_LIMIT:
      return {84, 1, false};
    case Sts3215Register::MAXIMUM_ACCELERATION:
      return {85, 1, true};
  }
  throw std::logic_error("Unknown STS3215 register.");
}

int decode_word(const std::uint8_t low, const std::uint8_t high)
{
  return static_cast<int>(low) | (static_cast<int>(high) << 8);
}

int decode_sign_magnitude(const int value, const int sign_bit)
{
  const int sign_mask = 1 << sign_bit;
  return (value & sign_mask) != 0 ? -(value & ~sign_mask) : value;
}

std::string communication_error(SMS_STS & bus, const std::string & operation)
{
  return operation + " failed (SDK communication error " +
         std::to_string(bus.getLastError()) + ", servo status " +
         std::to_string(bus.getState()) + ").";
}

}  // namespace

class FtServoTransport::Impl
{
public:
  SMS_STS bus;
  bool open = false;
  std::size_t sync_read_capacity = 0;
};

FtServoTransport::FtServoTransport()
: impl_(std::make_unique<Impl>())
{
}

FtServoTransport::~FtServoTransport()
{
  close();
}

void FtServoTransport::open(const std::string & port, int baud_rate)
{
  if (port.empty() || baud_rate <= 0)
  {
    throw std::invalid_argument("A serial port and positive baud rate are required.");
  }
  close();
  if (!impl_->bus.begin(baud_rate, port.c_str()))
  {
    throw std::runtime_error("Could not open STS3215 serial port " + port + ".");
  }
  impl_->open = true;
}

void FtServoTransport::close() noexcept
{
  if (impl_->open)
  {
    if (impl_->sync_read_capacity != 0)
    {
      impl_->bus.syncReadEnd();
      impl_->sync_read_capacity = 0;
    }
    impl_->bus.end();
    impl_->open = false;
  }
}

bool FtServoTransport::is_open() const noexcept
{
  return impl_->open;
}

int FtServoTransport::read_register(
  std::uint8_t servo_id, Sts3215Register register_name)
{
  if (!impl_->open)
  {
    throw std::logic_error("STS3215 serial port is not open.");
  }
  const auto info = register_info(register_name);
  const int result = info.width == 1 ?
    impl_->bus.readByte(servo_id, info.address) :
    impl_->bus.readWord(servo_id, info.address);
  if (result < 0)
  {
    throw std::runtime_error(
            communication_error(impl_->bus, "Reading servo " + std::to_string(servo_id)));
  }
  return result;
}

void FtServoTransport::write_register(
  std::uint8_t servo_id, Sts3215Register register_name, int value)
{
  if (!impl_->open)
  {
    throw std::logic_error("STS3215 serial port is not open.");
  }
  const auto info = register_info(register_name);
  if (!info.writable)
  {
    throw std::invalid_argument("Attempted to write a read-only STS3215 register.");
  }
  const int maximum = info.width == 1 ?
    std::numeric_limits<std::uint8_t>::max() :
    std::numeric_limits<std::uint16_t>::max();
  if (value < 0 || value > maximum)
  {
    throw std::out_of_range("STS3215 register value does not fit its wire width.");
  }
  const int result = info.width == 1 ?
    impl_->bus.writeByte(servo_id, info.address, static_cast<std::uint8_t>(value)) :
    impl_->bus.writeWord(servo_id, info.address, static_cast<std::uint16_t>(value));
  if (result != 1)
  {
    throw std::runtime_error(
            communication_error(impl_->bus, "Writing servo " + std::to_string(servo_id)));
  }
}

void FtServoTransport::set_eeprom_lock(
  const std::vector<std::uint8_t> & servo_ids, bool locked)
{
  if (!impl_->open)
  {
    throw std::logic_error("STS3215 serial port is not open.");
  }
  std::string first_error;
  for (const auto id : servo_ids)
  {
    const int result = locked ? impl_->bus.LockEprom(id) : impl_->bus.unLockEprom(id);
    if (result != 1)
    {
      const auto error = communication_error(
        impl_->bus, std::string(locked ? "Locking" : "Unlocking") +
        " servo " + std::to_string(id) + " EEPROM");
      if (!locked)
      {
        throw std::runtime_error(error);
      }
      if (first_error.empty())
      {
        first_error = error;
      }
    }
  }
  if (!first_error.empty())
  {
    throw std::runtime_error(first_error);
  }
}

std::map<std::uint8_t, Sts3215RawState> FtServoTransport::read_states(
  const std::vector<std::uint8_t> & servo_ids)
{
  if (!impl_->open)
  {
    throw std::logic_error("STS3215 serial port is not open.");
  }

  constexpr std::uint8_t kFirstAddress = 56;
  constexpr std::size_t kStateBytes = 15;
  if (servo_ids.empty() || servo_ids.size() > std::numeric_limits<std::uint8_t>::max())
  {
    throw std::invalid_argument("At least one STS3215 ID is required for state feedback.");
  }
  if (impl_->sync_read_capacity != servo_ids.size())
  {
    if (impl_->sync_read_capacity != 0)
    {
      impl_->bus.syncReadEnd();
    }
    impl_->bus.syncReadBegin(
      static_cast<std::uint8_t>(servo_ids.size()),
      static_cast<std::uint8_t>(kStateBytes), 20);
    impl_->sync_read_capacity = servo_ids.size();
  }

  auto ids = servo_ids;
  const int received = impl_->bus.syncReadPacketTx(
    ids.data(), static_cast<std::uint8_t>(ids.size()), kFirstAddress,
    static_cast<std::uint8_t>(kStateBytes));
  const int expected = static_cast<int>(servo_ids.size() * (kStateBytes + 6));
  if (received != expected)
  {
    throw std::runtime_error(
            communication_error(impl_->bus, "Reading synchronized STS3215 states"));
  }

  std::map<std::uint8_t, Sts3215RawState> states;
  for (const auto id : servo_ids)
  {
    std::array<std::uint8_t, kStateBytes> bytes{};
    const int count = impl_->bus.syncReadPacketRx(id, bytes.data());
    if (count != static_cast<int>(bytes.size()))
    {
      throw std::runtime_error(
              communication_error(
                impl_->bus, "Decoding synchronized state for servo " + std::to_string(id)));
    }

    const int velocity = decode_word(bytes[2], bytes[3]);
    const int current = decode_word(bytes[13], bytes[14]);
    states.emplace(
      id,
      Sts3215RawState{
        decode_word(bytes[0], bytes[1]),
        decode_sign_magnitude(velocity, 15),
        decode_sign_magnitude(current, 15),
        bytes[6],
        bytes[7],
        bytes[9],
      });
  }
  return states;
}

void FtServoTransport::write_positions(const std::map<std::uint8_t, int> & positions)
{
  if (!impl_->open)
  {
    throw std::logic_error("STS3215 serial port is not open.");
  }
  if (positions.empty() || positions.size() > std::numeric_limits<std::uint8_t>::max())
  {
    throw std::invalid_argument("At least one STS3215 position is required.");
  }

  std::vector<std::uint8_t> ids;
  std::vector<std::uint8_t> data;
  ids.reserve(positions.size());
  data.reserve(positions.size() * 2);
  for (const auto & [id, position] : positions)
  {
    if (position < 0 || position >= 4096)
    {
      throw std::out_of_range("STS3215 goal position must be in [0, 4095].");
    }
    ids.push_back(id);
    data.push_back(static_cast<std::uint8_t>(position & 0xff));
    data.push_back(static_cast<std::uint8_t>((position >> 8) & 0xff));
  }

  // Write only Goal_Position (42-43). The SDK's SyncWritePosEx helper starts at
  // Acceleration (41) and would also overwrite Goal_Velocity (46-47).
  impl_->bus.syncWrite(
    ids.data(), static_cast<std::uint8_t>(ids.size()), 42, data.data(), 2);
}

void FtServoTransport::set_torque(
  const std::vector<std::uint8_t> & servo_ids, bool enabled)
{
  if (!impl_->open)
  {
    throw std::logic_error("STS3215 serial port is not open.");
  }
  for (const auto id : servo_ids)
  {
    if (impl_->bus.EnableTorque(id, enabled ? 1 : 0) != 1)
    {
      throw std::runtime_error(
              communication_error(
                impl_->bus, std::string(enabled ? "Enabling" : "Disabling") +
                " torque on servo " + std::to_string(id)));
    }
  }
}

}  // namespace orion_hardware
