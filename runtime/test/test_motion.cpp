#include <gtest/gtest.h>

#include <map>
#include <string>
#include <vector>

#include "orion_runtime/joint_trajectory.hpp"
#include "orion_runtime/motion_library.hpp"
#include "orion_runtime/motion_sequence.hpp"
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

TEST(MotionLibraryTest, LoadsNestedFunctionalAndExpressiveMotions)
{
  const std::vector<std::string> joints = {
    "base_yaw_joint", "shoulder_pitch_joint", "elbow_pitch_joint",
    "head_roll_joint", "head_pitch_joint",
  };
  const PoseLibrary poses(ORION_TEST_POSES_FILE, joints);
  const MotionLibrary motions(ORION_TEST_MOTIONS_DIR, poses);

  EXPECT_EQ(motions.names().size(), 5U);
  const auto & right = motions.motion("look_at_right_expressive");
  ASSERT_EQ(right.keyframes.size(), 4U);
  EXPECT_EQ(right.keyframes.front().pose_name, "look_right_anticipation");
  EXPECT_DOUBLE_EQ(right.keyframes.front().duration_seconds, 2.00);
  EXPECT_EQ(right.keyframes.back().pose_name, "look_right");
  EXPECT_THROW(motions.motion("missing"), std::invalid_argument);
}

TEST(MotionSequenceTest, SamplesTransitionsAndAuthoredHolds)
{
  const MotionDefinition motion{
    "example",
    "",
    {
      {"first", {{"joint", 1.0}}, 2.0, 1.0},
      {"second", {{"joint", -1.0}}, 2.0, 0.5},
    },
  };
  const MotionSequence sequence(motion, {{"joint", 0.0}});

  EXPECT_DOUBLE_EQ(sequence.sample(0.0).at("joint"), 0.0);
  EXPECT_DOUBLE_EQ(sequence.sample(1.0).at("joint"), 0.5);
  EXPECT_DOUBLE_EQ(sequence.sample(2.5).at("joint"), 1.0);
  EXPECT_DOUBLE_EQ(sequence.sample(4.0).at("joint"), 0.0);
  EXPECT_DOUBLE_EQ(sequence.sample(5.5).at("joint"), -1.0);
  EXPECT_EQ(sequence.keyframe_name(2.5), "first");
  EXPECT_EQ(sequence.keyframe_name(3.0), "second");
  EXPECT_EQ(sequence.keyframe_index(3.0), 1U);
  EXPECT_EQ(sequence.keyframe_count(), 2U);
  EXPECT_DOUBLE_EQ(sequence.duration_seconds(), 5.5);
  EXPECT_TRUE(sequence.complete(5.5));
}

}  // namespace
}  // namespace orion_runtime
