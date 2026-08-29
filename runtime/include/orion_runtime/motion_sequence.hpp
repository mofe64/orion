#ifndef ORION_RUNTIME__MOTION_SEQUENCE_HPP_
#define ORION_RUNTIME__MOTION_SEQUENCE_HPP_

#include <cstddef>
#include <string>
#include <vector>

#include "orion_runtime/joint_trajectory.hpp"
#include "orion_runtime/motion_library.hpp"

namespace orion_runtime
{

class MotionSequence
{
public:
  MotionSequence(const MotionDefinition & motion, JointPositions start);

  JointPositions sample(double elapsed_seconds) const;
  double progress(double elapsed_seconds) const;
  bool complete(double elapsed_seconds) const;
  const std::string & name() const noexcept;
  const std::string & keyframe_name(double elapsed_seconds) const;
  std::size_t keyframe_index(double elapsed_seconds) const;
  std::size_t keyframe_count() const noexcept;
  double duration_seconds() const noexcept;

private:
  struct Segment
  {
    std::string pose_name;
    JointTrajectory transition;
    JointPositions target;
    double starts_at = 0.0;
    double arrives_at = 0.0;
    double holds_until = 0.0;
  };

  std::size_t segment_index(double elapsed_seconds) const;

  std::string name_;
  std::vector<Segment> segments_;
  double duration_seconds_ = 0.0;
};

}  // namespace orion_runtime

#endif  // ORION_RUNTIME__MOTION_SEQUENCE_HPP_
