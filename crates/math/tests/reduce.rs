//! Reducers: exactness of `ExactSum` and order-independence of every reducer.

#![forbid(unsafe_code)]

mod common;

use common::Rng;
use lactose_math::{ExactSum, IntSum, Max, Min, Reducer, Scalar, Vec3};

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn exact_sum(items: &[Scalar]) -> u64 {
    let mut sum = ExactSum::<Scalar>::identity();
    for &item in items {
        sum.add(item);
    }
    sum.finish().to_canonical_bits()
}

/// Folds `items` in a random order, split at random points into partials that are merged in
/// a random tree, with empty partials mixed in.
fn scrambled<R: Reducer>(rng: &mut Rng, items: &[R::Item]) -> R::Output
where
    R::Item: Copy,
{
    let mut items = items.to_vec();
    rng.shuffle(&mut items);

    let mut partials = vec![R::identity()];
    for item in items {
        if rng.below(4) == 0 {
            partials.push(R::identity());
        }
        // Sometimes leave the newest partial empty and add to an older one.
        let target = if rng.below(8) == 0 { rng.below(partials.len()) } else { partials.len() - 1 };
        partials[target].add(item);
    }
    for _ in 0..rng.below(3) {
        partials.push(R::identity());
    }

    while partials.len() > 1 {
        let from = partials.swap_remove(rng.below(partials.len()));
        let into = rng.below(partials.len());
        partials[into].merge(&from);
    }
    partials.pop().expect("one partial remains").finish()
}

fn random_list(rng: &mut Rng) -> Vec<Scalar> {
    let length = rng.below(200);
    match rng.below(4) {
        // Ordinary magnitudes.
        0 => (0..length).map(|_| rng.with_exponent(-8, 8)).collect(),
        // The whole finite range, where a running sum loses almost everything.
        1 => (0..length).map(|_| rng.with_exponent(-1022, 1023)).collect(),
        // Exact cancellation of large terms around small ones.
        2 => {
            let mut items: Vec<Scalar> =
                (0..length).map(|_| rng.with_exponent(-600, 600)).collect();
            let negated: Vec<Scalar> = items.iter().map(|&item| -item).collect();
            items.extend(negated);
            items.extend((0..rng.below(5)).map(|_| rng.with_exponent(-30, 30)));
            items
        }
        // Anything, special values included.
        _ => (0..length).map(|_| rng.any()).collect(),
    }
}

#[test]
fn exact_sum_is_independent_of_order_and_grouping() {
    let mut rng = Rng(0x0005_ca1a_b1e5);
    for round in 0..400 {
        let items = random_list(&mut rng);
        let expected = exact_sum(&items);
        for _ in 0..8 {
            let actual = scrambled::<ExactSum<Scalar>>(&mut rng, &items).to_canonical_bits();
            assert_eq!(
                actual,
                expected,
                "round {round}: a permutation and split of {} items changed the sum",
                items.len()
            );
        }
    }
}

#[test]
fn exact_sum_matches_integer_arithmetic() {
    // Items are integers scaled by 2^-20, each exactly representable, so the true sum is an
    // `i128` and its correctly rounded value is the integer-to-float conversion.
    let scale = s(1.0 / 1_048_576.0);
    let mut rng = Rng(0x1e_6e_12);
    for _ in 0..2_000 {
        let length = rng.below(64);
        let mut total: i128 = 0;
        let mut items = Vec::new();
        for _ in 0..length {
            let significand = (rng.next() >> 11) as i64 * if rng.below(2) == 0 { 1 } else { -1 };
            let integer = i128::from(significand) << rng.below(60);
            total += integer;
            // Exact: 53 significant bits, then scaling by a power of two.
            items.push(Scalar::from_f64(integer as f64) * scale);
        }
        let expected = (Scalar::from_f64(total as f64) * scale).to_canonical_bits();
        assert_eq!(exact_sum(&items), expected, "items {items:?}");
    }
}

#[test]
fn exact_sum_beats_a_running_sum() {
    assert_eq!(exact_sum(&[s(1e300), s(1.0), s(-1e300)]), s(1.0).to_canonical_bits());
    let tenth = [s(0.1); 10];
    // Ten times the float nearest 0.1 is nearer to one than to any other float; a running
    // sum gives 0.9999999999999999.
    assert_eq!(exact_sum(&tenth), s(1.0).to_canonical_bits());
}

#[test]
fn exact_sum_rounds_to_nearest_even() {
    let two_53 = s(9007199254740992.0);
    let tiny = Scalar::from_canonical_bits(1);
    // A tie goes to the even neighbour.
    assert_eq!(exact_sum(&[two_53, s(1.0)]), two_53.to_canonical_bits());
    assert_eq!(exact_sum(&[two_53, s(3.0)]), s(9007199254740996.0).to_canonical_bits());
    // Anything above the tie, however small, rounds up.
    assert_eq!(exact_sum(&[two_53, s(1.0), tiny]), s(9007199254740994.0).to_canonical_bits());
    // And just below the tie rounds down.
    assert_eq!(exact_sum(&[two_53, s(1.0), -tiny]), two_53.to_canonical_bits());
    // Negative sums round symmetrically.
    assert_eq!(exact_sum(&[-two_53, s(-1.0), -tiny]), s(-9007199254740994.0).to_canonical_bits());
    // A carry out of the fraction moves to the next binade.
    let below_two = Scalar::from_canonical_bits(0x3fff_ffff_ffff_ffff);
    assert_eq!(exact_sum(&[below_two, Scalar::EPSILON * s(0.5), tiny]), s(2.0).to_canonical_bits());
}

#[test]
fn exact_sum_range_limits() {
    let max = Scalar::MAX;
    let tiny = Scalar::from_canonical_bits(1);
    // Intermediate overflow is held exactly.
    assert_eq!(exact_sum(&[max, max, -max]), max.to_canonical_bits());
    assert_eq!(
        exact_sum(&[max; 1000].iter().chain(&[-max; 999]).copied().collect::<Vec<_>>()),
        max.to_canonical_bits()
    );
    // A final sum beyond the finite range rounds to infinity.
    assert_eq!(exact_sum(&[max, max]), Scalar::INFINITY.to_canonical_bits());
    assert_eq!(exact_sum(&[-max, -max]), Scalar::NEG_INFINITY.to_canonical_bits());
    // MAX plus half a unit in the last place is the first sum that rounds to infinity.
    let half_ulp_of_max = s(f64::from_bits(0x7c90_0000_0000_0000));
    assert_eq!(exact_sum(&[max, half_ulp_of_max]), Scalar::INFINITY.to_canonical_bits());
    assert_eq!(exact_sum(&[max, half_ulp_of_max, -tiny]), max.to_canonical_bits());
    // Subnormals add exactly, and across the boundary to normals.
    assert_eq!(exact_sum(&[tiny, tiny, tiny]), 3);
    assert_eq!(exact_sum(&[-tiny, -tiny]), 0x8000_0000_0000_0002);
    let largest_subnormal = Scalar::from_canonical_bits(0x000f_ffff_ffff_ffff);
    assert_eq!(exact_sum(&[largest_subnormal, tiny]), Scalar::MIN_POSITIVE.to_canonical_bits());
    assert_eq!(exact_sum(&[Scalar::MIN_POSITIVE, -tiny]), largest_subnormal.to_canonical_bits());
}

#[test]
fn exact_sum_zeros_and_special_values() {
    let nan = Scalar::NAN.to_canonical_bits();
    assert_eq!(exact_sum(&[]), 0);
    assert_eq!(exact_sum(&[-Scalar::ZERO, -Scalar::ZERO]), 0, "a zero sum is positive zero");
    assert_eq!(exact_sum(&[s(1.5), s(-1.5)]), 0);
    assert_eq!(exact_sum(&[s(1.0), Scalar::INFINITY]), Scalar::INFINITY.to_canonical_bits());
    assert_eq!(
        exact_sum(&[Scalar::NEG_INFINITY, s(1.0), Scalar::NEG_INFINITY]),
        Scalar::NEG_INFINITY.to_canonical_bits()
    );
    assert_eq!(exact_sum(&[Scalar::INFINITY, Scalar::NEG_INFINITY]), nan);
    assert_eq!(exact_sum(&[s(1.0), Scalar::NAN, Scalar::INFINITY]), nan);
    assert_eq!(exact_sum(&[s(f64::from_bits(0xfff0_0000_0000_0001))]), nan);
}

#[test]
fn exact_sum_of_vectors_is_lane_by_lane() {
    let mut rng = Rng(0x0003_1a9e);
    for _ in 0..100 {
        let items: Vec<Vec3> =
            (0..rng.below(60)).map(|_| Vec3::new(rng.any(), rng.any(), rng.any())).collect();
        let lanes = [0, 1, 2].map(|lane| {
            exact_sum(&items.iter().map(|item| item.to_array()[lane]).collect::<Vec<_>>())
        });
        let scrambled = scrambled::<ExactSum<Vec3>>(&mut rng, &items);
        assert_eq!(scrambled.to_array().map(Scalar::to_canonical_bits), lanes);
    }
}

#[test]
fn min_and_max_are_independent_of_order_and_grouping() {
    let mut rng = Rng(0x000e_17e3);
    for _ in 0..400 {
        // Draw from a small pool so ties, both zeros and NaN all occur often.
        let pool: Vec<Scalar> = (0..6).map(|_| rng.any()).collect();
        let items: Vec<Scalar> = (0..rng.below(40)).map(|_| pool[rng.below(pool.len())]).collect();

        let expected_min = items.iter().copied().reduce(Scalar::min);
        let expected_max = items.iter().copied().reduce(Scalar::max);
        for _ in 0..4 {
            let min = scrambled::<Min<Scalar>>(&mut rng, &items);
            let max = scrambled::<Max<Scalar>>(&mut rng, &items);
            assert_eq!(
                min.map(Scalar::to_canonical_bits),
                expected_min.map(Scalar::to_canonical_bits),
                "min of {items:?}"
            );
            assert_eq!(
                max.map(Scalar::to_canonical_bits),
                expected_max.map(Scalar::to_canonical_bits),
                "max of {items:?}"
            );
        }
    }
    assert_eq!(Min::<Scalar>::identity().finish(), None);
    assert_eq!(Max::<u32>::identity().finish(), None);
}

#[test]
fn integer_reducers_are_independent_of_order_and_grouping() {
    let mut rng = Rng(0x1257);
    for _ in 0..200 {
        let items: Vec<i64> = (0..rng.below(100)).map(|_| rng.next() as i64).collect();
        let total: i128 = items.iter().map(|&item| i128::from(item)).sum();
        assert_eq!(scrambled::<IntSum<i64>>(&mut rng, &items), total);
        assert_eq!(scrambled::<Min<i64>>(&mut rng, &items), items.iter().copied().min());
        assert_eq!(scrambled::<Max<i64>>(&mut rng, &items), items.iter().copied().max());
    }
    let mut unsigned = IntSum::<u64>::identity();
    unsigned.add(u64::MAX);
    unsigned.add(u64::MAX);
    assert_eq!(unsigned.finish(), 2 * i128::from(u64::MAX));
}
