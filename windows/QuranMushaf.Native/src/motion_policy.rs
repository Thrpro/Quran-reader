//! Bounded, portable policy for preparing original page pixels at idle.
//! The caller owns rendering, exact content keys and device resources.

pub const MAX_CPU_PIXEL_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TOTAL_PIXEL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RASTERS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterCandidate {
    pub page: u32,
    pub grayscale: bool,
}

/// Fixed storage avoids allocations and bounds every admission/iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterPlan {
    entries: [RasterCandidate; MAX_RASTERS],
    len: usize,
}

impl Default for RasterPlan {
    fn default() -> Self {
        Self {
            entries: [RasterCandidate {
                page: 0,
                grayscale: false,
            }; MAX_RASTERS],
            len: 0,
        }
    }
}

impl RasterPlan {
    pub fn as_slice(&self) -> &[RasterCandidate] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push(&mut self, candidate: RasterCandidate, capacity: usize) {
        if self.len < capacity && !self.as_slice().contains(&candidate) {
            self.entries[self.len] = candidate;
            self.len += 1;
        }
    }
}

/// Remaining CPU cache pixels after accounting for all active CPU rasters and
/// uploaded bitmap payloads. Saturation treats overflow as exhausted storage.
/// Driver allocations and native control snapshots have separate accounting.
pub fn spare_cache_budget(active_cpu_bytes: u64, active_gpu_bytes: u64) -> u64 {
    MAX_CPU_PIXEL_BYTES.saturating_sub(active_cpu_bytes).min(
        MAX_TOTAL_PIXEL_BYTES.saturating_sub(active_cpu_bytes.saturating_add(active_gpu_bytes)),
    )
}

/// Choose a stable desired set before any rendering allocation. The caller
/// warms only missing members, rather than continually evicting a desired key.
/// `previous`/`next` come from the reader's existing single-page/spread policy.
/// A clamped boundary redirects preference to the remaining real destination.
/// Selection, physical origin, DPI, palette, text mode and clock remain part of
/// the caller's exact cache key; these page/AA candidates never relax matching.
pub fn admit_rasters(
    current: u32,
    previous: u32,
    next: u32,
    preferred_direction: i32,
    bytes_per_raster: u64,
    budget: u64,
) -> RasterPlan {
    let mut plan = RasterPlan::default();
    if bytes_per_raster == 0
        || [current, previous, next]
            .iter()
            .any(|page| !(1..=604).contains(page))
    {
        return plan;
    }
    let capacity =
        (budget.min(MAX_CPU_PIXEL_BYTES) / bytes_per_raster).min(MAX_RASTERS as u64) as usize;
    if capacity == 0 {
        return plan;
    }
    let (mut preferred, mut other) = if preferred_direction < 0 {
        (previous, next)
    } else {
        (next, previous)
    };
    if preferred == current && other != current {
        std::mem::swap(&mut preferred, &mut other);
    }
    plan.push(
        RasterCandidate {
            page: current,
            grayscale: false,
        },
        capacity,
    );
    if preferred == current {
        return plan;
    }
    plan.push(
        RasterCandidate {
            page: preferred,
            grayscale: false,
        },
        capacity,
    );
    // With three slots, prepare a complete fold in the preferred direction.
    // With four/five, retain the other normal face before extra reverse faces.
    if capacity >= 4 && other != current {
        plan.push(
            RasterCandidate {
                page: other,
                grayscale: false,
            },
            capacity,
        );
    }
    plan.push(
        RasterCandidate {
            page: preferred,
            grayscale: true,
        },
        capacity,
    );
    if capacity >= 5 && other != current {
        plan.push(
            RasterCandidate {
                page: other,
                grayscale: true,
            },
            capacity,
        );
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn candidate(page: u32, grayscale: bool) -> RasterCandidate {
        RasterCandidate { page, grayscale }
    }

    #[test]
    fn spare_budget_counts_active_cpu_and_uploaded_pixels() {
        assert_eq!(spare_cache_budget(0, 0), 32 * MIB);
        assert_eq!(spare_cache_budget(24 * MIB, 24 * MIB), 8 * MIB);
        assert_eq!(spare_cache_budget(16 * MIB, 40 * MIB), 8 * MIB);
        assert_eq!(spare_cache_budget(33 * MIB, 0), 0);
        assert_eq!(spare_cache_budget(1, u64::MAX), 0);
        assert_eq!(spare_cache_budget(u64::MAX, u64::MAX), 0);
    }

    #[test]
    fn five_slots_warm_both_directions_and_prefer_requested_reverse_face() {
        let plan = admit_rasters(45, 43, 47, -1, 4 * MIB, 32 * MIB);
        assert_eq!(
            plan.as_slice(),
            &[
                candidate(45, false),
                candidate(43, false),
                candidate(47, false),
                candidate(43, true),
                candidate(47, true),
            ]
        );
    }

    #[test]
    fn four_slots_keep_both_normal_faces_and_preferred_grayscale() {
        let plan = admit_rasters(45, 43, 47, 1, 8 * MIB, 32 * MIB);
        assert_eq!(
            plan.as_slice(),
            &[
                candidate(45, false),
                candidate(47, false),
                candidate(43, false),
                candidate(47, true),
            ]
        );
    }

    #[test]
    fn three_slot_plan_is_stable_and_contains_only_one_complete_fold() {
        let expected = [
            candidate(45, false),
            candidate(47, false),
            candidate(47, true),
        ];
        for _ in 0..16 {
            let plan = admit_rasters(45, 43, 47, 1, 10 * MIB, 32 * MIB);
            assert_eq!(plan.as_slice(), &expected);
            assert_eq!(plan.len(), 3);
            assert!(plan.len() as u64 * 10 * MIB <= 32 * MIB);
        }
    }

    #[test]
    fn small_budgets_admit_only_available_slots() {
        assert!(admit_rasters(45, 43, 47, 1, 4 * MIB, 4 * MIB - 1).is_empty());
        assert_eq!(
            admit_rasters(45, 43, 47, 1, 4 * MIB, 4 * MIB).as_slice(),
            &[candidate(45, false)]
        );
        assert_eq!(
            admit_rasters(45, 43, 47, 1, 4 * MIB, 8 * MIB).as_slice(),
            &[candidate(45, false), candidate(47, false)]
        );
    }

    #[test]
    fn clamped_edges_do_not_prepare_nonexistent_turns_or_duplicate_keys() {
        assert_eq!(
            admit_rasters(1, 1, 3, -1, 4 * MIB, 32 * MIB).as_slice(),
            &[candidate(1, false), candidate(3, false), candidate(3, true)]
        );
        assert_eq!(
            admit_rasters(604, 603, 604, 1, 4 * MIB, 32 * MIB).as_slice(),
            &[
                candidate(604, false),
                candidate(603, false),
                candidate(603, true),
            ]
        );
        assert_eq!(
            admit_rasters(45, 43, 43, 1, 4 * MIB, 32 * MIB).as_slice(),
            &[
                candidate(45, false),
                candidate(43, false),
                candidate(43, true)
            ]
        );
        assert_eq!(
            admit_rasters(1, 1, 1, 1, 4 * MIB, 32 * MIB).as_slice(),
            &[candidate(1, false)]
        );
    }

    #[test]
    fn invalid_or_overflow_sized_requests_remain_empty_and_bounded() {
        assert!(admit_rasters(45, 43, 47, 1, 0, u64::MAX).is_empty());
        assert!(admit_rasters(45, 43, 47, 1, u64::MAX, u64::MAX).is_empty());
        assert!(admit_rasters(0, 1, 2, 1, 1, u64::MAX).is_empty());
        assert!(admit_rasters(604, 603, 605, 1, 1, u64::MAX).is_empty());
        let plan = admit_rasters(45, 43, 47, i32::MIN, 1, u64::MAX);
        assert_eq!(plan.len(), MAX_RASTERS);
        assert_eq!(plan.as_slice()[1], candidate(43, false));
        assert!(
            std::mem::size_of::<RasterPlan>()
                < MAX_RASTERS * std::mem::size_of::<RasterCandidate>()
                    + std::mem::size_of::<usize>()
                    + std::mem::align_of::<RasterPlan>()
        );
    }
}
