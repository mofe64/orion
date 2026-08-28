#include "orion_runtime/joint_trajectory.hpp"

#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <utility>

namespace orion_runtime
{

JointTrajectory::JointTrajectory(
  std::string name, JointPositions start, JointPositions target,
  double duration_seconds)
: name_(std::move(name)),
  start_(std::move(start)),
  target_(std::move(target)),
  duration_seconds_(duration_seconds)
{
  if (name_.empty() || !std::isfinite(duration_seconds_) || duration_seconds_ <= 0.0)
  {
    throw std::invalid_argument("Trajectory name and positive finite duration are required.");
  }
  if (start_.empty() || start_.size() != target_.size())
  {
    throw std::invalid_argument("Trajectory start and target joints must match.");
  }
  for (const auto & [joint_name, start] : start_)
  {
    const auto target = target_.find(joint_name);
    if (target == target_.end() || !std::isfinite(start) || !std::isfinite(target->second))
    {
      throw std::invalid_argument("Trajectory start and target joints must match and be finite.");
    }
  }
}

JointPositions JointTrajectory::sample(double elapsed_seconds) const
{
  const double phase = progress(elapsed_seconds);
  const double phase2 = phase * phase;
  const double phase3 = phase2 * phase;
  const double blend = phase3 * (10.0 + phase * (-15.0 + 6.0 * phase));

  JointPositions result;
  for (const auto & [joint_name, start] : start_)
  {
    const double target = target_.at(joint_name);
    result.emplace(joint_name, start + (target - start) * blend);
  }
  return result;
}

double JointTrajectory::progress(double elapsed_seconds) const
{
  if (!std::isfinite(elapsed_seconds))
  {
    throw std::invalid_argument("Trajectory elapsed time must be finite.");
  }
  return std::clamp(elapsed_seconds / duration_seconds_, 0.0, 1.0);
}

bool JointTrajectory::complete(double elapsed_seconds) const
{
  return progress(elapsed_seconds) >= 1.0;
}

const std::string & JointTrajectory::name() const noexcept
{
  return name_;
}

double JointTrajectory::duration_seconds() const noexcept
{
  return duration_seconds_;
}

}  // namespace orion_runtime
