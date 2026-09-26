//! 与实现独立的坐标/队列 oracle，所有测试至多 n=5。
use crate::bit_utils::*;
use crate::burnside::apply_burnside;
use crate::export::ExportManager;
use crate::fixed::enumerate_fixed_with_symmetry;
use crate::symmetric::*;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

fn cells(mask: u64) -> HashSet<(i32, i32)> {
    (0..64)
        .filter(|b| mask & (1 << b) != 0)
        .map(|b| (b / 8, b % 8))
        .collect()
}
fn normalize_oracle(points: &HashSet<(i32, i32)>) -> (u64, usize, usize) {
    let r0 = points.iter().map(|p| p.0).min().unwrap();
    let c0 = points.iter().map(|p| p.1).min().unwrap();
    let mut mask = 0;
    let mut w = 0;
    let mut h = 0;
    for &(r, c) in points {
        mask |= 1 << ((r - r0) * 8 + c - c0);
        w = w.max(c - c0 + 1);
        h = h.max(r - r0 + 1);
    }
    (mask, w as usize, h as usize)
}
fn adjacent((r, c): (i32, i32)) -> [(i32, i32); 4] {
    [(r - 1, c), (r + 1, c), (r, c - 1), (r, c + 1)]
}
fn connected(points: &HashSet<(i32, i32)>) -> bool {
    let start = *points.iter().next().unwrap();
    let mut visited = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        for q in adjacent(p) {
            if points.contains(&q) && visited.insert(q) {
                queue.push_back(q);
            }
        }
    }
    visited.len() == points.len()
}
fn hole_oracle(mask: u64, w: usize, h: usize) -> bool {
    let occupied = cells(mask);
    let mut visited = HashSet::from([(-1, -1)]);
    let mut queue = VecDeque::from([(-1, -1)]);
    while let Some(p) = queue.pop_front() {
        for (r, c) in adjacent(p) {
            if r >= -1
                && r <= h as i32
                && c >= -1
                && c <= w as i32
                && !occupied.contains(&(r, c))
                && visited.insert((r, c))
            {
                queue.push_back((r, c));
            }
        }
    }
    (0..h as i32)
        .any(|r| (0..w as i32).any(|c| !occupied.contains(&(r, c)) && !visited.contains(&(r, c))))
}
fn canonical_oracle(mask: u64) -> u64 {
    let mut points = cells(mask);
    let mut best = u64::MAX;
    for _ in 0..4 {
        best = best.min(normalize_oracle(&points).0);
        points = points.into_iter().map(|(r, c)| (c, -r)).collect();
    }
    best
}
fn fixed_oracle() -> HashSet<u64> {
    let mut result = HashSet::new();
    for packed in 1u64..1 << 16 {
        let mut m = 0;
        for r in 0..4 {
            m |= ((packed >> (r * 4)) & 15) << (r * 8);
        }
        let points = cells(m);
        if connected(&points) {
            result.insert(normalize_oracle(&points).0);
        }
    }
    result
}

#[test]
fn all_four_by_four_shapes_match_coordinate_oracles() {
    let fixed = fixed_oracle();
    assert_eq!(fixed.len(), 9472);
    for m in fixed {
        let p = cells(m);
        let (_, w, h) = normalize_oracle(&p);
        assert_eq!(mask_extent(m), (w, h));
        assert_eq!(normalize_translation(m << 9), (m, w, h));
        assert_eq!(compute_canonical(m, w, h).0, canonical_oracle(m));
        assert_eq!(poly_has_hole(m, w, h), hole_oracle(m, w, h), "mask {m:x}");
        let rot: HashSet<_> = p.iter().map(|&(r, c)| (c, -r)).collect();
        let r90 = normalize_oracle(&rot).0;
        assert_eq!(rotate90(m, w, h).0, r90);
        assert_eq!(has_symmetry_90(m, w, h), m == r90);
        let r180: HashSet<_> = rot.iter().map(|&(r, c)| (c, -r)).collect();
        assert_eq!(has_symmetry_180(m, w, h), m == normalize_oracle(&r180).0);
        for q in p
            .iter()
            .flat_map(|&p| adjacent(p))
            .filter(|q| !p.contains(q))
        {
            let mut grown = p.clone();
            grown.insert(q);
            let expected = normalize_oracle(&grown);
            if expected.1 <= 4 && expected.2 <= 4 {
                assert_eq!(grow_mask(m, w, h, q.0, q.1), expected);
            }
        }
    }
}

#[test]
fn bfs_known_counts_through_five() {
    let (fixed, s90, s180) = enumerate_fixed_with_symmetry(5, false, None).unwrap();
    let counts = apply_burnside(&fixed, &s90, &s180);
    let expected = [
        (1, 1, 0),
        (4, 4, 0),
        (46, 44, 2),
        (2404, 1899, 505),
        (520818, 267976, 252842),
    ];
    for (r, (total, no_hole, has_hole)) in counts.iter().zip(expected) {
        assert_eq!((r.total, r.no_hole, r.has_hole), (total, no_hole, has_hole));
    }
}

#[test]
fn canonical_bfs_matches_fixed_orbits_and_transfer_through_five() {
    let (fixed, s90, s180) = enumerate_fixed_with_symmetry(5, false, None).unwrap();
    let (canonical, a90, a180) = crate::fixed::enumerate_canonical(5, false, None).unwrap();
    for (actual, expected) in [(&canonical, &fixed), (&a90, &s90), (&a180, &s180)] {
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!(
                (a.total, a.no_hole, a.has_hole),
                (e.total, e.no_hole, e.has_hole)
            );
        }
    }
    let one_sided = apply_burnside(&canonical, &a90, &a180);
    for (a, e) in one_sided
        .iter()
        .zip(crate::transfer::enumerate_transfer(5, false))
    {
        assert_eq!(
            (a.total, a.no_hole, a.has_hole),
            (e.total, e.no_hole, e.has_hole)
        );
    }
}

fn read_masks(dir: &std::path::Path) -> Vec<u64> {
    let mut result = Vec::new();
    if !dir.exists() {
        return result;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "bin") {
            let data = std::fs::read(path).unwrap();
            assert_eq!(data.len() % 8, 0);
            result.extend(
                data.chunks_exact(8)
                    .map(|b| u64::from_le_bytes(b.try_into().unwrap())),
            );
        }
    }
    result
}

#[test]
fn export_preserves_an_existing_dataset() {
    let dir = std::env::temp_dir().join(format!("room-count-existing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("dataset-id.txt");
    std::fs::write(&marker, b"existing dataset").unwrap();
    let refused = ExportManager::new(&dir).is_err();
    assert_eq!(std::fs::read(&marker).unwrap(), b"existing dataset");
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(refused, "必须拒绝重用非空目录，保护现有global_index");
}

#[test]
fn export_matches_complete_four_by_four_oracle() {
    check_export(false);
    check_export(true);
}

fn check_export(canonical: bool) {
    let dir = std::env::temp_dir().join(format!(
        "room-count-regression-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mgr = Arc::new(ExportManager::new(&dir).unwrap());
    if canonical {
        crate::fixed::enumerate_canonical(4, false, Some(mgr.clone())).unwrap();
    } else {
        enumerate_fixed_with_symmetry(4, false, Some(mgr.clone())).unwrap();
    }
    mgr.flush_all().unwrap();
    drop(mgr);
    let expected: HashSet<_> = fixed_oracle().into_iter().map(canonical_oracle).collect();
    let all = read_masks(&dir.join("all_fixed"));
    assert_eq!(all.len(), 2404);
    assert_eq!(all.iter().copied().collect::<HashSet<_>>(), expected);
    let mut classified = HashSet::new();
    for (name, hole) in [("no_holes", false), ("with_holes", true)] {
        for md in 1..=4 {
            for m in read_masks(&dir.join(name).join(format!("n{md:02}_fixed"))) {
                let (_, w, h) = normalize_oracle(&cells(m));
                assert_eq!(w.max(h), md);
                assert_eq!(hole_oracle(m, w, h), hole);
                assert!(classified.insert(m));
            }
        }
    }
    assert_eq!(classified, expected);
    std::fs::remove_dir_all(dir).unwrap();
}
