#include "orion_runtime/motion_sequence.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <utility>

namespace orion_runtime
{

MotionSequence::MotionSequence(const MotionDefinition & motion, JointPositions start)
: name_(motion.name)
{
  if (name_.empty() || motion.keyframes.empty() || start.empty())
  {
    throw std::invalid_argument("Motion sequence requires a name, start, and keyframes.");
  }

  JointPositions segment_start = std::move(start);
  double starts_at = 0.0;
  for (const auto & keyframe : motion.keyframes)
  {
    const double arrives_at = starts_at + keyframe.duration_seconds;
    const double holds_until = arrives_at + keyframe.hold_seconds;
    segments_.push_back(Segment{
      keyframe.pose_name,
      JointTrajectory(
        motion.name + ":" + keyframe.pose_name,
        segment_start,
        keyframe.target,
        keyframe.duration_seconds),
      keyframe.target,
      starts_at,
      arrives_at,
      holds_until,
    });
    segment_start = keyframe.target;
    starts_at = holds_until;
  }
  duration_seconds_ = starts_at;
}

JointPositions MotionSequence::sample(double elapsed_seconds) const
{
  const auto & segment = segments_.at(segment_index(elapsed_seconds));
  if (elapsed_seconds < segment.arrives_at)
  {
    return segment.transition.sample(elapsed_seconds - segment.starts_at);
  }
  return segment.target;
}

double MotionSequence::progress(double elapsed_seconds) const
{
  if (!std::isfinite(elapsed_seconds))
  {
    throw std::invalid_argument("Motion elapsed time must be finite.");
  }
  return std::clamp(elapsed_seconds / duration_seconds_, 0.0, 1.0);
}

bool MotionSequence::complete(double elapsed_seconds) const
{
  return progress(elapsed_seconds) >= 1.0;
}

const std::string & MotionSequence::name() const noexcept
{
  return name_;
}

const std::string & MotionSequence::keyframe_name(double elapsed_seconds) const
{
  return segments_.at(segment_index(elapsed_seconds)).pose_name;
}

std::size_t MotionSequence::keyframe_index(double elapsed_seconds) const
{
  return segment_index(elapsed_seconds);
}

std::size_t MotionSequence::keyframe_count() const noexcept
{
  return segments_.size();
}

double MotionSequence::duration_seconds() const noexcept
{
  return duration_seconds_;
}

std::size_t MotionSequence::segment_index(double elapsed_seconds) const
{
  if (!std::isfinite(elapsed_seconds))
  {
    throw std::invalid_argument("Motion elapsed time must be finite.");
  }
  const double clamped = std::max(0.0, elapsed_seconds);
  for (std::size_t index = 0; index < segments_.size(); ++index)
  {
    if (clamped < segments_[index].holds_until || index + 1 == segments_.size())
    {
      return index;
    }
  }
  throw std::logic_error("Motion sequence contains no segment.");
}

}  // namespace orion_runtime
