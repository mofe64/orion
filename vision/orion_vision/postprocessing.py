"""Bounded NumPy non-maximum suppression for frame-pixel xywh boxes."""
import numpy as np


def nms(boxes, scores, threshold, maximum, candidate_limit=1000):
    order = np.argsort(-scores, kind='stable')[:candidate_limit]
    kept = []
    while len(order) and len(kept) < maximum:
        index, rest = order[0], order[1:]
        kept.append(int(index))
        xy = np.maximum(boxes[index, :2], boxes[rest, :2])
        end = np.minimum(boxes[index, :2] + boxes[index, 2:], boxes[rest, :2] + boxes[rest, 2:])
        intersection = np.maximum(end - xy, 0).prod(axis=1)
        union = boxes[index, 2:].prod() + boxes[rest, 2:].prod(axis=1) - intersection
        order = rest[intersection / np.maximum(union, 1e-12) <= threshold]
    return kept
