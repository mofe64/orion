#ifndef ORION_HARDWARE__FTSERVO_TRANSPORT_HPP_
#define ORION_HARDWARE__FTSERVO_TRANSPORT_HPP_

#include <memory>

#include "orion_hardware/sts3215_transport.hpp"

namespace orion_hardware
{

/// STS3215 transport backed by Feetech's official Linux serial SDK.
class FtServoTransport final : public Sts3215Transport
{
public:
  FtServoTransport();
  ~FtServoTransport() override;

  FtServoTransport(const FtServoTransport &) = delete;
  FtServoTransport & operator=(const FtServoTransport &) = delete;

  void open(const std::string & port, int baud_rate) override;
  void close() noexcept override;
  bool is_open() const noexcept override;

  int read_register(std::uint8_t servo_id, Sts3215Register register_name) override;
  void write_register(
    std::uint8_t servo_id, Sts3215Register register_name, int value) override;
  void set_eeprom_lock(
    const std::vector<std::uint8_t> & servo_ids, bool locked) override;
  std::map<std::uint8_t, Sts3215RawState> read_states(
    const std::vector<std::uint8_t> & servo_ids) override;
  void write_positions(const std::map<std::uint8_t, int> & positions) override;
  void set_torque(
    const std::vector<std::uint8_t> & servo_ids, bool enabled) override;

private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace orion_hardware

#endif  // ORION_HARDWARE__FTSERVO_TRANSPORT_HPP_
