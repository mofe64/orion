#include <gtest/gtest.h>

#include <map>
#include <string>
#include <vector>

#include "orion_runtime/joint_trajectory.hpp"
#include "orion_runtime/pose_library.hpp"

namespace orion_runtime
{
namespace
{

TEST(JointTrajectoryTest, UsesQuinticEndpointsAndMidpoint)
{
  const JointTrajectory trajectory(
    "test", {{"a", 0.0}, {"b", 1.0}}, {{"a", 1.0}, {"b", -1.0}}, 2.0);

  EXPECT_DOUBLE_EQ(trajectory.sample(0.0).at("a"), 0.0);
  EXPECT_DOUBLE_EQ(trajectory.sample(1.0).at("a"), 0.5);
  EXPECT_DOUBLE_EQ(trajectory.sample(1.0).at("b"), 0.0);
  EXPECT_DOUBLE_EQ(trajectory.sample(2.0).at("a"), 1.0);
  EXPECT_TRUE(trajectory.complete(2.0));
}

TEST(JointTrajectoryTest, RejectsMismatchedTargetsAndInvalidDuration)
{
  EXPECT_THROW(
    JointTrajectory("bad", {{"a", 0.0}}, {{"b", 1.0}}, 1.0),
    std::invalid_argument);
  EXPECT_THROW(
    JointTrajectory("bad", {{"a", 0.0}}, {{"a", 1.0}}, 0.0),
    std::invalid_argument);
}

TEST(PoseLibraryTest, LoadsOrionNamedPoses)
{
  const std::vector<std::string> joints = {
    "base_yaw_joint", "shoulder_pitch_joint", "elbow_pitch_joint",
    "head_roll_joint", "head_pitch_joint",
  };
  const PoseLibrary poses(ORION_TEST_POSES_FILE, joints);

  EXPECT_EQ(poses.pose("rest").size(), 5U);
  EXPECT_DOUBLE_EQ(poses.pose("home").at("shoulder_pitch_joint"), 0.0);
  EXPECT_THROW(poses.pose("missing"), std::invalid_argument);
}

}  // namespace
}  // namespace orion_runtime
