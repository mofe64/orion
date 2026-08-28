#include <gtest/gtest.h>

#include <string>
#include <vector>

#include "orion_runtime/state_snapshot.hpp"

namespace orion_runtime
{
namespace
{

TEST(StateSnapshotTest, SerializesVersionedObserveModeContract)
{
  const StateSnapshot snapshot{
    RuntimeMode::OBSERVE,
    42,
    123456789,
    50.0,
    std::vector<orion_hardware::JointState>{
      {"base_yaw_joint", -0.078, 0.0, 0.0, 6.2, 29.0, 0},
      {"head_\"pitch_joint", 0.124, 0.1, 13.0, 6.1, 30.0, 2},
    },
  };

  const std::string json = state_snapshot_to_json(snapshot);

  EXPECT_NE(json.find("\"schema_version\":1"), std::string::npos);
  EXPECT_NE(json.find("\"robot\":\"orion\""), std::string::npos);
  EXPECT_NE(json.find("\"mode\":\"observe\""), std::string::npos);
  EXPECT_NE(json.find("\"profile_applied\":false"), std::string::npos);
  EXPECT_NE(json.find("\"torque_enabled\":false"), std::string::npos);
  EXPECT_NE(json.find("\"sequence\":42"), std::string::npos);
  EXPECT_NE(json.find("\"sampled_at_unix_ns\":123456789"), std::string::npos);
  EXPECT_NE(json.find("\"update_hz\":50"), std::string::npos);
  EXPECT_NE(json.find("\"motion\":null"), std::string::npos);
  EXPECT_NE(json.find("\"name\":\"head_\\\"pitch_joint\""), std::string::npos);
  EXPECT_NE(json.find("\"position_rad\":-0.078"), std::string::npos);
  EXPECT_NE(json.find("\"status\":2"), std::string::npos);
}

TEST(StateSnapshotTest, StampsSnapshotsWithUnixTime)
{
  const auto snapshot = make_state_snapshot(RuntimeMode::CONFIGURED, 1, 50.0, {});

  EXPECT_EQ(snapshot.mode, RuntimeMode::CONFIGURED);
  EXPECT_EQ(snapshot.sequence, 1U);
  EXPECT_EQ(snapshot.update_hz, 50.0);
  EXPECT_GT(snapshot.sampled_at_unix_ns, 0);
}

TEST(StateSnapshotTest, DerivesLifecycleFlagsFromMode)
{
  EXPECT_FALSE(profile_is_applied(RuntimeMode::OBSERVE));
  EXPECT_FALSE(torque_is_enabled(RuntimeMode::OBSERVE));
  EXPECT_TRUE(profile_is_applied(RuntimeMode::CONFIGURED));
  EXPECT_FALSE(torque_is_enabled(RuntimeMode::CONFIGURED));
  EXPECT_TRUE(profile_is_applied(RuntimeMode::HOLDING));
  EXPECT_TRUE(torque_is_enabled(RuntimeMode::HOLDING));
  EXPECT_TRUE(profile_is_applied(RuntimeMode::MOVING));
  EXPECT_TRUE(torque_is_enabled(RuntimeMode::MOVING));
  EXPECT_STREQ(runtime_mode_name(RuntimeMode::MOVING), "moving");
  EXPECT_STREQ(runtime_mode_name(RuntimeMode::HOLDING), "holding");
}

TEST(StateSnapshotTest, SerializesActiveMotion)
{
  const auto snapshot = make_state_snapshot(
    RuntimeMode::MOVING, 8, 50.0, {}, "home", 0.25);
  const auto json = state_snapshot_to_json(snapshot);

  EXPECT_NE(json.find("\"mode\":\"moving\""), std::string::npos);
  EXPECT_NE(json.find("\"motion\":{\"name\":\"home\",\"progress\":0.25}"),
    std::string::npos);
}

}  // namespace
}  // namespace orion_runtime
