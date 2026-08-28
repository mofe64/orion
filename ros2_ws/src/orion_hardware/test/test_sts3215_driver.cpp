#include <gtest/gtest.h>

#include <algorithm>
#include <cstdint>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "orion_hardware/sts3215_driver.hpp"

namespace orion_hardware
{
namespace
{

struct RegisterKey
{
  std::uint8_t id;
  Sts3215Register register_name;

  bool operator<(const RegisterKey & other) const
  {
    return std::tie(id, register_name) < std::tie(other.id, other.register_name);
  }
};

class FakeTransport final : public Sts3215Transport
{
public:
  void open(const std::string & port, int baud_rate) override
  {
    calls.push_back("open:" + port + ":" + std::to_string(baud_rate));
    open_ = true;
  }

  void close() noexcept override
  {
    if (open_)
    {
      calls.push_back("close");
    }
    open_ = false;
  }

  bool is_open() const noexcept override {return open_;}

  int read_register(std::uint8_t id, Sts3215Register register_name) override
  {
    calls.push_back("read_register");
    return registers.at(RegisterKey{id, register_name});
  }

  void write_register(
    std::uint8_t id, Sts3215Register register_name, int value) override
  {
    calls.push_back("write_register");
    register_writes.emplace_back(id, register_name, value);
    registers[RegisterKey{id, register_name}] = value;
  }

  void set_eeprom_lock(const std::vector<std::uint8_t> &, bool locked) override
  {
    calls.push_back(locked ? "eeprom_lock" : "eeprom_unlock");
  }

  std::map<std::uint8_t, Sts3215RawState> read_states(
    const std::vector<std::uint8_t> &) override
  {
    calls.push_back("read_states");
    return states;
  }

  void write_positions(const std::map<std::uint8_t, int> & values) override
  {
    calls.push_back("write_positions");
    position_writes.push_back(values);
    for (const auto & [id, position] : values)
    {
      registers[RegisterKey{id, Sts3215Register::GOAL_POSITION}] = position;
    }
  }

  void set_torque(const std::vector<std::uint8_t> &, bool enabled) override
  {
    calls.push_back(enabled ? "torque_on" : "torque_off");
  }

  void add_servo(std::uint8_t id, int position)
  {
    const std::map<Sts3215Register, int> baseline = {
      {Sts3215Register::MODEL_NUMBER, 777},
      {Sts3215Register::FIRMWARE_MAJOR_VERSION, 3},
      {Sts3215Register::FIRMWARE_MINOR_VERSION, 10},
      {Sts3215Register::RETURN_DELAY_TIME, 0},
      {Sts3215Register::PHASE, 12},
      {Sts3215Register::P_COEFFICIENT, 32},
      {Sts3215Register::I_COEFFICIENT, 0},
      {Sts3215Register::D_COEFFICIENT, 32},
      {Sts3215Register::OPERATING_MODE, 0},
      {Sts3215Register::TORQUE_ENABLE, 0},
      {Sts3215Register::ACCELERATION, 0},
      {Sts3215Register::GOAL_POSITION, position},
      {Sts3215Register::GOAL_VELOCITY, 0},
      {Sts3215Register::TORQUE_LIMIT, 1000},
      {Sts3215Register::STATUS, 0},
      {Sts3215Register::MAXIMUM_ACCELERATION, 50},
    };
    for (const auto & [register_name, value] : baseline)
    {
      registers[RegisterKey{id, register_name}] = value;
    }
    states[id] = Sts3215RawState{position, 0, 0, 62, 28, 0};
  }

  bool open_ = false;
  std::vector<std::string> calls;
  std::map<RegisterKey, int> registers;
  std::map<std::uint8_t, Sts3215RawState> states;
  std::vector<std::tuple<std::uint8_t, Sts3215Register, int>> register_writes;
  std::vector<std::map<std::uint8_t, int>> position_writes;
};

std::vector<JointCalibration> calibrations()
{
  return {
    {"base_yaw_joint", 1, 942, 1, -1004, 1004},
    {"head_pitch_joint", 5, 3476, 1, -385, 1145},
  };
}

std::size_t call_index(const std::vector<std::string> & calls, const std::string & value)
{
  const auto found = std::find(calls.begin(), calls.end(), value);
  if (found == calls.end())
  {
    throw std::runtime_error("Expected fake-transport call was not recorded: " + value);
  }
  return static_cast<std::size_t>(std::distance(calls.begin(), found));
}

TEST(Sts3215DriverTest, AppliesOnlyLeLampProfileDifferences)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 904);
  transport->add_servo(5, 3547);
  Sts3215Driver driver(transport);

  driver.configure("/dev/fake", 1000000, calibrations());

  EXPECT_EQ(transport->registers.at({1, Sts3215Register::P_COEFFICIENT}), 16);
  EXPECT_EQ(transport->registers.at({5, Sts3215Register::P_COEFFICIENT}), 16);
  EXPECT_EQ(transport->registers.at({1, Sts3215Register::MAXIMUM_ACCELERATION}), 254);
  EXPECT_EQ(transport->registers.at({5, Sts3215Register::MAXIMUM_ACCELERATION}), 254);
  EXPECT_EQ(transport->registers.at({1, Sts3215Register::ACCELERATION}), 254);
  EXPECT_EQ(transport->registers.at({5, Sts3215Register::ACCELERATION}), 254);

  for (const auto & [id, register_name, value] : transport->register_writes)
  {
    (void)id;
    (void)value;
    EXPECT_NE(register_name, Sts3215Register::GOAL_VELOCITY);
    EXPECT_NE(register_name, Sts3215Register::TORQUE_LIMIT);
  }
  EXPECT_LT(
    call_index(transport->calls, "eeprom_unlock"),
    call_index(transport->calls, "eeprom_lock"));
}

TEST(Sts3215DriverTest, ConnectsAndReadsWithoutWritingServoState)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 904);
  transport->add_servo(5, 3547);
  Sts3215Driver driver(transport);

  driver.connect("/dev/fake", 1000000, calibrations());
  const auto states = driver.read();

  ASSERT_EQ(states.size(), 2U);
  EXPECT_TRUE(transport->register_writes.empty());
  EXPECT_TRUE(transport->position_writes.empty());
  EXPECT_EQ(
    std::count(transport->calls.begin(), transport->calls.end(), "eeprom_unlock"), 0);
  EXPECT_EQ(
    std::count(transport->calls.begin(), transport->calls.end(), "eeprom_lock"), 0);
  EXPECT_EQ(
    std::count(transport->calls.begin(), transport->calls.end(), "torque_on"), 0);
  EXPECT_EQ(
    std::count(transport->calls.begin(), transport->calls.end(), "torque_off"), 0);
}

TEST(Sts3215DriverTest, RefusesActivationUntilServoProfileIsApplied)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 904);
  transport->add_servo(5, 3547);
  Sts3215Driver driver(transport);

  driver.connect("/dev/fake", 1000000, calibrations());

  EXPECT_THROW(driver.activate(), std::logic_error);
  EXPECT_TRUE(transport->position_writes.empty());
}

TEST(Sts3215DriverTest, SeedsPresentPositionsBeforeTorqueOn)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 904);
  transport->add_servo(5, 3547);
  Sts3215Driver driver(transport);
  driver.configure("/dev/fake", 1000000, calibrations());
  transport->calls.clear();

  const auto states = driver.activate();

  ASSERT_EQ(states.size(), 2U);
  ASSERT_EQ(transport->position_writes.size(), 1U);
  EXPECT_EQ(transport->position_writes.front().at(1), 904);
  EXPECT_EQ(transport->position_writes.front().at(5), 3547);
  EXPECT_LT(
    call_index(transport->calls, "read_states"),
    call_index(transport->calls, "write_positions"));
  EXPECT_LT(
    call_index(transport->calls, "write_positions"),
    call_index(transport->calls, "torque_on"));
}

TEST(Sts3215DriverTest, ConvertsCommandsAcrossEncoderWrap)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 942);
  transport->add_servo(5, 32);
  Sts3215Driver driver(transport);
  driver.configure("/dev/fake", 1000000, calibrations());
  const auto initial = driver.activate();

  EXPECT_NEAR(initial.at(1).position, 1.0, 0.002);
  driver.write({{"base_yaw_joint", 0.0}, {"head_pitch_joint", 1.0}});

  ASSERT_EQ(transport->position_writes.size(), 2U);
  EXPECT_EQ(transport->position_writes.back().at(5), 32);
  EXPECT_THROW(
    driver.write({{"base_yaw_joint", 0.0}, {"head_pitch_joint", 2.0}}),
    std::out_of_range);
}

TEST(Sts3215DriverTest, RefusesConfigurationWhileTorqueIsOn)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 904);
  transport->add_servo(5, 3547);
  transport->registers[{1, Sts3215Register::TORQUE_ENABLE}] = 1;
  Sts3215Driver driver(transport);

  EXPECT_THROW(
    driver.configure("/dev/fake", 1000000, calibrations()),
    std::runtime_error);
  EXPECT_FALSE(transport->is_open());
  EXPECT_TRUE(transport->register_writes.empty());
}

TEST(Sts3215DriverTest, RefusesConfigurationWhenServoReportsFault)
{
  auto transport = std::make_shared<FakeTransport>();
  transport->add_servo(1, 904);
  transport->add_servo(5, 3547);
  transport->registers[{5, Sts3215Register::STATUS}] = 4;
  Sts3215Driver driver(transport);

  EXPECT_THROW(
    driver.configure("/dev/fake", 1000000, calibrations()),
    std::runtime_error);
  EXPECT_FALSE(transport->is_open());
  EXPECT_TRUE(transport->register_writes.empty());
}

}  // namespace
}  // namespace orion_hardware
