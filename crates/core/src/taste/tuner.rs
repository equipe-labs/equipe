//! The taste tuner: the person picks the one they like more of two real
//! product screens, again and again — about twenty picks in two minutes give
//! a first style profile, more refine it — and equipe works out where they
//! lean on each axis of style ([`TasteAxis`]).
//!
//! - The set, `design/tuner/set.json`, is the designer's: pictures from the
//!   project's references and its finds, each placed on the axes, varied one
//!   or two at a time (`{"items": [{"picture": "references/a.png", "source":
//!   "Product — Pricing", "axes": {"density": -60, "contrast": 20}}]}`).
//! - The pairs: each varies one or two axes and holds the rest, the axes
//!   asked least first ([`next_pairs`]); one pair in six is a contrast probe,
//!   deliberately off the profile, to test the edges of the taste.
//! - What the person did, `design/tuner/signals.jsonl`, a line each: picks
//!   (with one-tap reasons), and the implicit signals — a chosen option or a
//!   board's hearted tile liked, a passed-over option or another round
//!   disliked, work sent back and pins with their reasons ([`Record`]).
//! - The leans ([`leans`]): every signal votes on the axes its pictures
//!   differ on; a lean is the votes' balance, its confidence how many there
//!   were and how much they agree.
//!
//! Pure but for reading and writing those two files.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::design::folder;
use crate::protocol::design::{Lean, PickReason, PickSide, StyleProfile, TasteAxis, TunerItem};

pub const SET_FILE: &str = "tuner/set.json";
pub const SIGNALS_FILE: &str = "tuner/signals.jsonl";
/// The most pictures a set holds.
pub const SET_MAX: usize = 200;
/// Two pictures differ on an axis this far apart or more, out of 200 ...
pub const DIFF: i32 = 40;
/// ... and hold it within this.
pub const SAME: i32 = 30;
/// One pair in this many is a contrast probe.
pub const PROBE_EVERY: usize = 6;
/// The most signal lines read.
const SIGNALS_MAX: usize = 5000;

/// One line of `signals.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    /// A tuner pick (`chose` none: skipped).
    Pick {
        a: String,
        b: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chose: Option<PickSide>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        reasons: Vec<PickReason>,
        #[serde(default)]
        date: String,
    },
    /// A picture liked or disliked by what the person did, not by a pick:
    /// `weight` in percent of a pick's.
    Like {
        picture: String,
        liked: bool,
        weight: u8,
        #[serde(default)]
        date: String,
    },
    /// Work turned down, a reaction, or a pin, with its one-tap reasons and
    /// the person's words.
    Reasons {
        what: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        reasons: Vec<PickReason>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        words: String,
        #[serde(default)]
        date: String,
    },
}

/// How much an implicit signal weighs beside a pick, in percent.
pub const CHOSEN: u8 = 50;
pub const HEARTED: u8 = 50;
pub const PASSED_OVER: u8 = 25;
pub const REACTED: u8 = 60;

#[derive(Deserialize)]
struct SetFile {
    #[serde(default)]
    items: Vec<SetItem>,
}

#[derive(Deserialize)]
struct SetItem {
    picture: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    axes: std::collections::BTreeMap<String, f64>,
}

/// An axis value as written: -100..100, or -1..1.
fn value(v: f64) -> i8 {
    let scaled = if v.abs() <= 1.0 && v.fract() != 0.0 { v * 100.0 } else { v };
    scaled.round().clamp(-100.0, 100.0) as i8
}

/// The set in `text`, its pictures as `dir` resolves them (inside it only;
/// one outside, missing or not a picture left out).
pub fn parse_set(text: &str, dir: &Path) -> Result<Vec<TunerItem>, String> {
    let file: SetFile = serde_json::from_str(text).map_err(|e| format!("{SET_FILE}: {e}"))?;
    let mut out = Vec::new();
    for item in file.items.into_iter().take(SET_MAX) {
        if !crate::visuals::is_image_name(&item.picture) {
            continue;
        }
        // Inside the folder only (no `..`, no link out), and named as the folder names it.
        if std::path::Path::new(&item.picture).components().any(|c| !matches!(c, std::path::Component::Normal(_))) || folder::within(dir, &item.picture).is_none() {
            continue;
        }
        let path = dir.join(&item.picture);
        let axes = TasteAxis::ALL.iter().filter_map(|a| item.axes.iter().find(|(k, _)| TasteAxis::parse(k) == Some(*a)).map(|(_, v)| (*a, value(*v)))).collect();
        out.push(TunerItem { picture: path.to_string_lossy().into_owned(), source: item.source.trim().to_owned(), axes });
    }
    Ok(out)
}

/// The set in the design folder `dir`, if it has one.
pub fn read_set(dir: &Path) -> Vec<TunerItem> {
    let Some(bytes) = folder::inside(dir, SET_FILE, 1 << 20) else { return Vec::new() };
    parse_set(&String::from_utf8_lossy(&bytes), dir).unwrap_or_default()
}

/// What the set says of itself, for `equipe design tuner check`: problems
/// that keep it from tuning well. Empty: ready.
pub fn check(items: &[TunerItem]) -> Vec<String> {
    let mut out = Vec::new();
    if items.len() < 8 {
        out.push(format!("{} pictures; at least 8 make enough pairs (24 or more is better)", items.len()));
    }
    for axis in TasteAxis::ALL {
        let (lo, hi) = items.iter().map(|i| i.at(axis)).fold((0i8, 0i8), |(l, h), v| (l.min(v), h.max(v)));
        if i32::from(hi) - i32::from(lo) < DIFF {
            out.push(format!("nothing varies {}: place some pictures towards {} and some towards {}", axis.key(), axis.ends().0, axis.ends().1));
        }
    }
    let focused = (0..items.len()).flat_map(|i| (i + 1..items.len()).map(move |j| (i, j))).filter(|&(i, j)| focused(&items[i], &items[j])).count();
    if items.len() >= 2 && focused < 6 {
        out.push(format!("only {focused} pairs vary one or two axes and hold the rest; add pictures that differ from another in one way"));
    }
    out
}

/// The axes two pictures differ on.
pub fn differ(a: &TunerItem, b: &TunerItem) -> Vec<TasteAxis> {
    TasteAxis::ALL.into_iter().filter(|x| (i32::from(a.at(*x)) - i32::from(b.at(*x))).abs() >= DIFF).collect()
}

/// A pair that varies one or two axes and holds the others.
fn focused(a: &TunerItem, b: &TunerItem) -> bool {
    let d = differ(a, b);
    (1..=2).contains(&d.len()) && TasteAxis::ALL.iter().filter(|x| !d.contains(x)).all(|x| (i32::from(a.at(*x)) - i32::from(b.at(*x))).abs() <= SAME)
}

/// A pair to show: the pictures by their place in the set, and whether it is
/// a contrast probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub a: usize,
    pub b: usize,
    pub probe: bool,
}

/// A tie broken the same way every time for the same seed.
fn mix(i: usize, j: usize, seed: u64) -> u64 {
    let mut x = seed ^ (((i as u64) << 32) | j as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^ (x >> 33)
}

/// How well a picture fits the profile's leans: positive near it.
fn fit(item: &TunerItem, leans: &[Lean]) -> i64 {
    leans.iter().map(|l| i64::from(l.lean) * i64::from(l.confidence) * i64::from(item.at(l.axis))).sum()
}

/// The next `n` pairs after `done` picks: never one already `tuned` (either
/// way round), each varying one or two axes, the axes asked least first and
/// the pictures shown least; every [`PROBE_EVERY`]th a contrast probe — the
/// pair whose fit to the `leans` differs most (one near the profile, one off
/// it), or with no leans yet the pair that differs on the most axes.
pub fn next_pairs(items: &[TunerItem], tuned: &[(String, String)], leans: &[Lean], done: usize, n: usize, seed: u64) -> Vec<Pair> {
    let index = |p: &str| items.iter().position(|i| i.picture == p);
    let mut used: Vec<(usize, usize)> = tuned.iter().filter_map(|(a, b)| Some((index(a)?, index(b)?))).map(|(a, b)| (a.min(b), a.max(b))).collect();
    let mut cover = [0usize; 6];
    let mut shown = vec![0usize; items.len()];
    for &(a, b) in &used {
        for axis in differ(&items[a], &items[b]) {
            cover[axis as usize] += 1;
        }
        shown[a] += 1;
        shown[b] += 1;
    }
    let mut out = Vec::new();
    for k in 0..n {
        let probe = (done + k) % PROBE_EVERY == PROBE_EVERY - 1;
        let fresh = (0..items.len()).flat_map(|i| (i + 1..items.len()).map(move |j| (i, j))).filter(|p| !used.contains(p));
        let pick = if probe {
            fresh
                .filter(|&(i, j)| !differ(&items[i], &items[j]).is_empty())
                .max_by_key(|&(i, j)| {
                    let spread = if leans.is_empty() { differ(&items[i], &items[j]).len() as i64 } else { (fit(&items[i], leans) - fit(&items[j], leans)).abs() };
                    (spread, std::cmp::Reverse(mix(i, j, seed)))
                })
        } else {
            let candidates: Vec<(usize, usize)> = fresh.filter(|&(i, j)| !differ(&items[i], &items[j]).is_empty()).collect();
            let any_focused = candidates.iter().any(|&(i, j)| focused(&items[i], &items[j]));
            candidates.into_iter().filter(|&(i, j)| !any_focused || focused(&items[i], &items[j])).min_by_key(|&(i, j)| {
                let axes = differ(&items[i], &items[j]);
                let least = axes.iter().map(|x| cover[*x as usize]).max().unwrap_or(0);
                (least, shown[i] + shown[j], mix(i, j, seed))
            })
        };
        let Some((a, b)) = pick else { break };
        for axis in differ(&items[a], &items[b]) {
            cover[axis as usize] += 1;
        }
        shown[a] += 1;
        shown[b] += 1;
        used.push((a, b));
        // Which side each picture shows on is the seed's, not the set's order.
        let (a, b) = if mix(b, a, seed).is_multiple_of(2) { (a, b) } else { (b, a) };
        out.push(Pair { a, b, probe });
    }
    out
}

/// The votes one reason gives by itself: what it says of the thing turned
/// down, as (axis, direction) — "too dense" leans airy, "too plain" leans
/// ornate and colourful.
fn reason_votes(reason: PickReason) -> &'static [(TasteAxis, f64)] {
    match reason {
        PickReason::TooDense => &[(TasteAxis::Density, -1.0)],
        PickReason::TooPlain => &[(TasteAxis::Ornament, 1.0), (TasteAxis::Color, 0.5)],
        _ => &[],
    }
}

/// The axis a reason says mattered in a pick: its vote there counts double.
fn reason_axis(reason: PickReason) -> Option<TasteAxis> {
    match reason {
        PickReason::Colors => Some(TasteAxis::Color),
        PickReason::Type => Some(TasteAxis::TypeScale),
        PickReason::Layout => Some(TasteAxis::Layout),
        PickReason::TooDense => Some(TasteAxis::Density),
        _ => None,
    }
}

/// Where the person leans on each axis, from what they did, and how many
/// picks that is. Pictures not in the set (`items`) vote on nothing.
pub fn leans(items: &[TunerItem], records: &[Record]) -> (Vec<Lean>, u32) {
    let find = |p: &str| items.iter().find(|i| i.picture == p);
    // Per axis: the votes' sum, and their weight.
    let mut sum = [0f64; 6];
    let mut weight = [0f64; 6];
    let mut vote = |axis: TasteAxis, v: f64, w: f64| {
        sum[axis as usize] += v * w;
        weight[axis as usize] += w;
    };
    let mut picks = 0u32;
    for record in records {
        match record {
            Record::Pick { a, b, chose: Some(side), reasons, .. } => {
                picks += 1;
                let (Some(a), Some(b)) = (find(a), find(b)) else { continue };
                let (won, lost) = if *side == PickSide::A { (a, b) } else { (b, a) };
                for axis in differ(won, lost) {
                    let gap = (f64::from(won.at(axis)) - f64::from(lost.at(axis))) / 100.0;
                    let w = if reasons.iter().any(|r| reason_axis(*r) == Some(axis)) { 2.0 } else { 1.0 };
                    vote(axis, gap.signum() * gap.abs().min(1.0), w);
                }
                for (axis, v) in reasons.iter().flat_map(|r| reason_votes(*r)) {
                    vote(*axis, *v, 0.5);
                }
            }
            Record::Pick { chose: None, .. } => {}
            Record::Like { picture, liked, weight: w, .. } => {
                let Some(item) = find(picture) else { continue };
                let sign = if *liked { 1.0 } else { -1.0 };
                for axis in TasteAxis::ALL {
                    let at = f64::from(item.at(axis)) / 100.0;
                    if at.abs() >= 0.2 {
                        vote(axis, sign * at.signum(), f64::from(*w) / 100.0 * at.abs());
                    }
                }
            }
            Record::Reasons { reasons, .. } => {
                for (axis, v) in reasons.iter().flat_map(|r| reason_votes(*r)) {
                    vote(*axis, *v, 0.5);
                }
            }
        }
    }
    let out = TasteAxis::ALL
        .into_iter()
        .filter(|a| weight[*a as usize] > 0.0)
        .map(|axis| {
            let (s, w) = (sum[axis as usize], weight[axis as usize]);
            let agree = (s.abs() / w).min(1.0);
            let amount = 1.0 - 1.0 / (1.0 + w / 3.0);
            Lean { axis, lean: (100.0 * s / w).round().clamp(-100.0, 100.0) as i8, confidence: (100.0 * amount * (0.5 + 0.5 * agree)).round().clamp(0.0, 100.0) as u8 }
        })
        .collect();
    (out, picks)
}

/// The pairs already picked or skipped.
pub fn tuned(records: &[Record]) -> Vec<(String, String)> {
    records.iter().filter_map(|r| if let Record::Pick { a, b, .. } = r { Some((a.clone(), b.clone())) } else { None }).collect()
}

/// The signals kept in the design folder `dir`, oldest first.
pub fn read_signals(dir: &Path) -> Vec<Record> {
    let Some(bytes) = folder::inside(dir, SIGNALS_FILE, 8 << 20) else { return Vec::new() };
    String::from_utf8_lossy(&bytes).lines().filter_map(|l| serde_json::from_str(l).ok()).take(SIGNALS_MAX).collect()
}

/// Add `records` to the design folder `dir`'s signals.
pub fn append(dir: &Path, records: &[Record]) -> Result<(), String> {
    use std::io::Write as _;
    if records.is_empty() {
        return Ok(());
    }
    let path = dir.join(SIGNALS_FILE);
    if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(format!("{} isn't a plain file", path.display()));
    }
    std::fs::create_dir_all(dir.join("tuner")).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
    for r in records {
        let line = serde_json::to_string(r).map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What a designer's answered questions say without a word: each picked
/// option's pictures liked, each passed-over option's disliked (lightly), all
/// of them disliked on "another round", and a board's hearted tiles liked.
pub fn from_answers(set: &crate::protocol::decide::QuestionSet, answers: &[crate::protocol::decide::Answer], today: &str) -> Vec<Record> {
    use crate::protocol::decide::Reaction;
    let like = |picture: &str, liked: bool, weight: u8| Record::Like { picture: picture.to_owned(), liked, weight, date: today.to_owned() };
    let mut out = Vec::new();
    for (question, answer) in set.questions.iter().zip(answers) {
        let again = matches!(answer.reaction, Some(Reaction::Again));
        if !again && answer.picked.is_empty() && answer.liked.is_empty() {
            continue;
        }
        for (i, option) in question.options.iter().enumerate() {
            let picked = answer.picked.contains(&i) && !again;
            let pictures = option.images.iter().chain(option.tiles.iter().map(|t| &t.image).take(if question.board { 0 } else { usize::MAX }));
            for picture in pictures {
                out.push(if picked { like(picture, true, CHOSEN) } else { like(picture, false, PASSED_OVER) });
            }
        }
        for heart in &answer.liked {
            if let Some(tile) = question.options.get(heart.option).and_then(|o| o.tiles.get(heart.tile)) {
                out.push(like(&tile.image, true, HEARTED));
            }
        }
    }
    out
}

/// `profile` with its leans and picks worked out again from `items` and `records`.
pub fn refresh(profile: &mut StyleProfile, items: &[TunerItem], records: &[Record]) {
    let (leans, picks) = leans(items, records);
    profile.leans = leans;
    profile.picks = picks;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, axes: &[(TasteAxis, i8)]) -> TunerItem {
        TunerItem { picture: format!("/d/{name}.png"), source: name.into(), axes: axes.to_vec() }
    }

    /// For each axis, a low and a high picture that hold every other axis.
    fn balanced() -> Vec<TunerItem> {
        let mut out = vec![item("base", &[])];
        for axis in TasteAxis::ALL {
            out.push(item(&format!("{}-lo", axis.key()), &[(axis, -70)]));
            out.push(item(&format!("{}-hi", axis.key()), &[(axis, 70)]));
        }
        out
    }

    fn pick(a: &TunerItem, b: &TunerItem, side: PickSide, reasons: &[PickReason]) -> Record {
        Record::Pick { a: a.picture.clone(), b: b.picture.clone(), chose: Some(side), reasons: reasons.to_vec(), date: "2026-11-16".into() }
    }

    #[test]
    fn pairs_vary_one_or_two_axes_and_cover_every_axis_evenly() {
        let items = balanced();
        let pairs = next_pairs(&items, &[], &[], 0, 12, 7);
        assert_eq!(pairs.len(), 12);
        let mut cover = [0usize; 6];
        for p in pairs.iter().filter(|p| !p.probe) {
            let d = differ(&items[p.a], &items[p.b]);
            assert!((1..=2).contains(&d.len()), "{:?} vs {:?}", items[p.a].source, items[p.b].source);
            for a in d {
                cover[a as usize] += 1;
            }
        }
        let (lo, hi) = (cover.iter().min().unwrap(), cover.iter().max().unwrap());
        assert!(*lo >= 1 && hi - lo <= 1, "even: {cover:?}");
        // One in six is a probe: here the 6th and the 12th.
        let probes: Vec<usize> = pairs.iter().enumerate().filter(|(_, p)| p.probe).map(|(i, _)| i).collect();
        assert_eq!(probes, [5, 11]);
        // Never the same pair twice, in a batch or after one.
        let mut seen: Vec<(usize, usize)> = pairs.iter().map(|p| (p.a.min(p.b), p.a.max(p.b))).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 12);
        let tuned: Vec<(String, String)> = pairs.iter().map(|p| (items[p.b].picture.clone(), items[p.a].picture.clone())).collect();
        let more = next_pairs(&items, &tuned, &[], 12, 6, 7);
        assert!(more.iter().all(|p| !seen.contains(&(p.a.min(p.b), p.a.max(p.b)))));
        assert_eq!(next_pairs(&items, &[], &[], 0, 12, 7), pairs, "the same seed, the same pairs");
    }

    #[test]
    fn a_probe_pairs_one_picture_near_the_profile_with_one_off_it() {
        let items = vec![
            item("airy-light", &[(TasteAxis::Density, -80), (TasteAxis::Contrast, -60)]),
            item("airy-dark", &[(TasteAxis::Density, -80), (TasteAxis::Contrast, 60)]),
            item("dense-light", &[(TasteAxis::Density, 80), (TasteAxis::Contrast, -60)]),
            item("dense-dark", &[(TasteAxis::Density, 80), (TasteAxis::Contrast, 60), (TasteAxis::Color, 90)]),
        ];
        let leans = [Lean { axis: TasteAxis::Density, lean: -70, confidence: 80 }, Lean { axis: TasteAxis::Contrast, lean: -40, confidence: 60 }];
        let pairs = next_pairs(&items, &[], &leans, 5, 1, 1);
        assert!(pairs[0].probe);
        let names = [items[pairs[0].a].source.as_str(), items[pairs[0].b].source.as_str()];
        assert!(names.contains(&"airy-light") && names.contains(&"dense-dark"), "{names:?}");
        // With no profile yet, the pair that differs most.
        let first = next_pairs(&items, &[], &[], 5, 1, 1);
        assert_eq!(differ(&items[first[0].a], &items[first[0].b]).len(), 3);
    }

    #[test]
    fn picks_for_airy_pages_lean_airy_and_grow_surer() {
        let items = balanced();
        let (lo, hi) = (&items[1], &items[2]);
        let one = vec![pick(lo, hi, PickSide::A, &[])];
        let (l1, n1) = leans(&items, &one);
        assert_eq!(n1, 1);
        assert_eq!(l1, [Lean { axis: TasteAxis::Density, lean: -100, confidence: 25 }]);
        let five: Vec<Record> = (0..5).map(|_| pick(hi, lo, PickSide::B, &[])).collect();
        let (l5, _) = leans(&items, &five);
        assert_eq!(l5[0].lean, -100);
        assert!(l5[0].confidence > l1[0].confidence, "more picks, surer: {l5:?}");
        // Picks that disagree: a weaker lean, a lower confidence than as many that agree.
        let mixed: Vec<Record> = (0..5).map(|n| pick(lo, hi, if n < 3 { PickSide::A } else { PickSide::B }, &[])).collect();
        let (lm, _) = leans(&items, &mixed);
        assert_eq!(lm[0].lean, -20);
        assert!(lm[0].confidence < l5[0].confidence);
        // A skip counts as no pick and moves nothing.
        let skip = vec![Record::Pick { a: lo.picture.clone(), b: hi.picture.clone(), chose: None, reasons: vec![], date: String::new() }];
        assert_eq!(leans(&items, &skip), (vec![], 0));
    }

    #[test]
    fn reasons_and_implicit_signals_move_the_profile_too() {
        let items = balanced();
        let color_lo = &items[5];
        let color_hi = &items[6];
        // "The colours": the pick's colour vote counts double.
        let (plain, _) = leans(&items, &[pick(color_lo, color_hi, PickSide::A, &[])]);
        let (said, _) = leans(&items, &[pick(color_lo, color_hi, PickSide::A, &[PickReason::Colors])]);
        assert!(said[0].confidence > plain[0].confidence);
        // "Too dense" on work turned down leans airy with no picture at all.
        let (dense, picks) = leans(&items, &[Record::Reasons { what: "Hero".into(), reasons: vec![PickReason::TooDense], words: String::new(), date: String::new() }]);
        assert_eq!(picks, 0);
        assert_eq!(dense[0].axis, TasteAxis::Density);
        assert!(dense[0].lean < 0);
        // A hearted colourful tile leans colourful; a passed-over one the other way.
        let liked = Record::Like { picture: color_hi.picture.clone(), liked: true, weight: HEARTED, date: String::new() };
        let (l, _) = leans(&items, &[liked]);
        assert_eq!((l[0].axis, l[0].lean > 0), (TasteAxis::Color, true));
        let passed = Record::Like { picture: color_hi.picture.clone(), liked: false, weight: PASSED_OVER, date: String::new() };
        assert!(leans(&items, &[passed]).0[0].lean < 0);
        // A picture not in the set says nothing.
        assert!(leans(&items, &[Record::Like { picture: "/elsewhere.png".into(), liked: true, weight: 50, date: String::new() }]).0.is_empty());
    }

    #[test]
    fn chosen_passed_over_and_hearted_pictures_become_signals() {
        use crate::protocol::decide::{Answer, Choice, Like, Question, QuestionSet, Reaction, Tile};
        let option = |name: &str| Choice { label: name.into(), images: vec![format!("/d/{name}.png")], ..Choice::default() };
        let pick_set = QuestionSet { id: 1, session: 1, member: "designer".into(), ticket: None, questions: vec![Question { question: "Hero?".into(), options: vec![option("a"), option("b")], ..Question::default() }], asked_at: 0, review: false, retro: None };
        let got = from_answers(&pick_set, &[Answer::pick(1)], "d");
        assert_eq!(got, [Record::Like { picture: "/d/a.png".into(), liked: false, weight: PASSED_OVER, date: "d".into() }, Record::Like { picture: "/d/b.png".into(), liked: true, weight: CHOSEN, date: "d".into() }]);
        let again = Answer { reaction: Some(Reaction::Again), ..Answer::default() };
        assert!(from_answers(&pick_set, &[again], "d").iter().all(|r| matches!(r, Record::Like { liked: false, .. })), "another round: none liked");
        assert!(from_answers(&pick_set, &[Answer::skipped()], "d").is_empty(), "skipped says nothing");
        let tiles = vec![Tile { image: "/d/t0.png".into(), ..Tile::default() }, Tile { image: "/d/t1.png".into(), ..Tile::default() }];
        let board = QuestionSet { questions: vec![Question { board: true, options: vec![Choice { label: "fam".into(), tiles, ..Choice::default() }], ..Question::default() }], ..pick_set };
        let hearted = Answer { picked: vec![0], liked: vec![Like { option: 0, tile: 1 }], ..Answer::default() };
        assert_eq!(from_answers(&board, &[hearted], "d"), [Record::Like { picture: "/d/t1.png".into(), liked: true, weight: HEARTED, date: "d".into() }]);
    }

    struct Dir(std::path::PathBuf);
    impl Dir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_set_is_read_inside_its_folder_and_checked_for_balance() {
        let root = std::env::temp_dir().join(format!("equipe-tuner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("references")).unwrap();
        let dir = Dir(root);
        for n in ["a", "b"] {
            std::fs::write(dir.path().join(format!("references/{n}.png")), b"x").unwrap();
        }
        let text = r#"{"items":[
            {"picture":"references/a.png","source":"A","axes":{"density":-0.6,"Type scale":80}},
            {"picture":"references/b.png","axes":{"colour":250}},
            {"picture":"../outside.png"},
            {"picture":"references/missing.png"},
            {"picture":"notes.md"}]}"#;
        let set = parse_set(text, dir.path()).unwrap();
        assert_eq!(set.len(), 2, "outside, missing and non-pictures left out");
        assert_eq!(set[0].axes, [(TasteAxis::Density, -60), (TasteAxis::TypeScale, 80)]);
        assert_eq!(set[1].at(TasteAxis::Color), 100);
        assert!(parse_set("not json", dir.path()).is_err());
        let problems = check(&set);
        assert!(problems.iter().any(|p| p.contains("at least 8")), "{problems:?}");
        assert!(check(&balanced()).is_empty(), "{:?}", check(&balanced()));
        // Signals go on, a line each, and come back.
        let records = [pick(&set[0], &set[1], PickSide::A, &[PickReason::NotUs])];
        append(dir.path(), &records).unwrap();
        append(dir.path(), &records).unwrap();
        assert_eq!(read_signals(dir.path()).len(), 2);
        assert_eq!(tuned(&read_signals(dir.path()))[0], (set[0].picture.clone(), set[1].picture.clone()));
    }
}
