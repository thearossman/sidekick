use clap::{Parser, ValueEnum};
use log::{debug, info, warn};
use multiset::HashMultiSet;
use quack::{
    arithmetic::{ModularArithmetic, ModularInteger},
    *,
};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use sha2::{Digest, Sha256};
use std::fmt::{Debug, Display};
use std::ops::{AddAssign, MulAssign, Sub, SubAssign};
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
pub struct QuackParams {
    /// The threshold number of dropped packets.
    #[arg(long, short = 't', default_value_t = 20)]
    threshold: usize,
    /// Number of identifier bits.
    #[arg(long = "bits", short = 'b', default_value_t = 32)]
    num_bits_id: usize,
    /// Enable pre-computation optimization
    #[arg(long)]
    precompute: bool,
    /// Disable not-factoring optimization
    #[arg(long)]
    factor: bool,
    /// Enable Montgomery multiplication optimization
    #[arg(long)]
    montgomery: bool,
}

#[derive(Parser, Debug)]
struct Cli {
    /// Quack type.
    #[arg(value_enum)]
    quack_ty: QuackType,
    /// Number of trials.
    #[arg(long = "trials", default_value_t = 10)]
    num_trials: usize,
    /// Number of sent packets.
    #[arg(short = 'n', default_value_t = 1000)]
    num_packets: usize,
    /// Number of dropped packets.
    #[arg(short = 'd', long = "dropped", default_value_t = 20)]
    num_drop: usize,
    /// Number of connections.
    #[arg(short = 'c', long = "connections", default_value_t = 1)]
    num_conns: usize,
    /// Quack parameters.
    #[command(flatten)]
    quack: QuackParams,
}

#[derive(Clone, ValueEnum, Debug, PartialEq, Eq)]
pub enum QuackType {
    Strawman1a,
    Strawman1b,
    Strawman2,
    PowerSum,
}

pub fn print_summary(d: Vec<Duration>, num_packets: usize) {
    let size = d.len() as u32;
    let avg = if d.is_empty() {
        Duration::new(0, 0)
    } else {
        d.into_iter().sum::<Duration>() / size
    };
    warn!("SUMMARY: num_trials = {}, avg = {:?}", size, avg);
    let d_per_packet = avg / num_packets as u32;
    let ns_per_packet = d_per_packet.as_secs() * 1000000000 + d_per_packet.subsec_nanos() as u64;
    let packets_per_s = 1000000000 / ns_per_packet;
    warn!(
        "SUMMARY (per-packet): {:?}/packet = {} packets/s",
        d_per_packet, packets_per_s
    )
}

pub fn gen_numbers<T>(num_packets: usize) -> Vec<T>
where
    Standard: Distribution<T>,
{
    (0..num_packets).map(|_| rand::thread_rng().gen()).collect()
}

fn benchmark_decode_strawman1a(num_packets: usize, num_drop: usize) -> Duration {
    let numbers = gen_numbers::<u32>(num_packets);

    // Construct two empty Quacks.
    let mut acc1 = HashMultiSet::new();
    let mut acc2 = HashMultiSet::new();

    // Insert all but num_drop random numbers into the second accumulator.
    for &number in numbers.iter().take(num_packets - num_drop) {
        acc2.insert(number);
    }

    let t1 = Instant::now();
    // Insert all random numbers into the first accumulator.
    // Then find the set difference.
    for &number in numbers.iter().take(num_packets) {
        acc1.insert(number);
    }
    let dropped = acc1 - acc2;
    let t2 = Instant::now();

    let duration = t2 - t1;
    info!(
        "Decode time (num_packets={}, \
        false_positives = {}, dropped = {}): {:?}",
        num_packets,
        dropped.len() - num_drop,
        num_drop,
        duration
    );
    assert_eq!(dropped.len(), num_drop);
    duration
}

const NUM_SUBSETS_LIMIT: u32 = 1000000;

fn benchmark_decode_strawman2(num_packets: usize, num_drop: usize) -> Duration {
    let numbers = gen_numbers::<u32>(num_packets);
    let mut acc1 = Sha256::new();

    // Insert all but num_drop random numbers into the accumulator.
    for number in numbers.iter().take(num_packets - num_drop) {
        acc1.update(number.to_be_bytes());
    }
    acc1.finalize();

    // Calculate the number of subsets.
    let _n = num_packets as u32;
    let _r = num_drop as u32;
    // let num_subsets = (n-r+1..=n).product();

    let t1 = Instant::now();
    if num_drop > 0 {
        // For every subset of size "num_packets - num_drop"
        // Calculate the SHA256 hash
        // let num_hashes_to_calculate = std::cmp::min(
        //     NUM_SUBSETS_LIMIT, num_subsets / 2);
        let num_hashes_to_calculate = NUM_SUBSETS_LIMIT;

        // We're really just measuring a lower bound of the time to compute
        // any SHA256 hash with this number of elements
        for _ in 0..num_hashes_to_calculate {
            let mut acc2 = Sha256::new();
            for number in numbers.iter().take(num_packets - num_drop) {
                acc2.update(number.to_be_bytes());
            }
            acc2.finalize();
        }
    }
    let t2 = Instant::now();

    let duration = t2 - t1;
    info!("Decode time (num_packets={}): {:?}", num_packets, duration);
    info!(
        "Calculated {} hashes, expected {}C{}",
        NUM_SUBSETS_LIMIT, num_packets, num_drop
    );

    duration
}

fn benchmark_decode_power_sum_factor_u32(
    size: usize,
    num_packets: usize,
    num_drop: usize,
) -> Duration {
    let numbers = gen_numbers::<u32>(num_packets);

    // Construct two empty Quacks.
    let mut acc1 = PowerSumQuack::<u32>::new(size);
    let mut acc2 = PowerSumQuack::<u32>::new(size);

    // Insert all but num_drop random numbers into the second accumulator.
    for &number in numbers.iter().take(num_packets - num_drop) {
        acc2.insert(number);
    }

    let t1 = Instant::now();
    for &number in numbers.iter().take(num_packets) {
        acc1.insert(number);
    }
    acc1 -= acc2;
    let dropped = acc1.decode_by_factorization().unwrap();
    let t2 = Instant::now();

    let duration = t2 - t1;
    info!(
        "Decode time (bits = 32, threshold = {}, num_packets={}, \
        false_positives = {}, dropped = {}): {:?}",
        size,
        num_packets,
        dropped.len() - num_drop,
        num_drop,
        duration
    );
    assert_eq!(dropped.len(), num_drop);
    duration
}

fn benchmark_decode_power_sum_precompute_u16(
    size: usize,
    num_packets: usize,
    num_drop: usize,
) -> Duration {
    let numbers = gen_numbers::<u16>(num_packets);

    // Construct two empty Quacks.
    let mut acc1 = PowerTableQuack::new(size);
    let mut acc2 = PowerTableQuack::new(size);

    // Insert all but num_drop random numbers into the second accumulator.
    for &number in numbers.iter().take(num_packets - num_drop) {
        acc2.insert(number);
    }

    let t1 = Instant::now();
    for &number in numbers.iter().take(num_packets) {
        acc1.insert(number);
    }
    acc1 -= acc2;
    let dropped = acc1.decode_with_log(&numbers);
    let t2 = Instant::now();

    let duration = t2 - t1;
    info!(
        "Decode time (bits = 32, threshold = {}, num_packets={}, \
        false_positives = {}, dropped = {}): {:?}",
        size,
        num_packets,
        dropped.len() - num_drop,
        num_drop,
        duration
    );
    assert!(dropped.len() >= num_drop);
    duration
}

fn benchmark_decode_power_sum_montgomery_u64(
    size: usize,
    num_packets: usize,
    num_drop: usize,
) -> Duration {
    let numbers = gen_numbers::<u64>(num_packets);

    // Construct two empty Quacks.
    let mut acc1 = MontgomeryQuack::new(size);
    let mut acc2 = MontgomeryQuack::new(size);

    // Insert all but num_drop random numbers into the second accumulator.
    for &number in numbers.iter().take(num_packets - num_drop) {
        acc2.insert(number);
    }

    let t1 = Instant::now();
    for &number in numbers.iter().take(num_packets) {
        acc1.insert(number);
    }
    acc1 -= acc2;
    let dropped = acc1.decode_with_log(&numbers);
    let t2 = Instant::now();

    let duration = t2 - t1;
    info!(
        "Decode time (bits = 64, threshold = {}, num_packets={}, \
        false_positives = {}, dropped = {}): {:?}",
        size,
        num_packets,
        dropped.len() - num_drop,
        num_drop,
        duration
    );
    assert!(dropped.len() >= num_drop);
    duration
}

fn benchmark_decode_power_sum<T>(
    size: usize,
    num_bits_id: usize,
    num_packets: usize,
    num_drop: usize,
) -> Duration
where
    Standard: Distribution<T>,
    T: Debug + Display + Default + PartialOrd + Sub<Output = T> + Copy,
    ModularInteger<T>: ModularArithmetic<T> + AddAssign + MulAssign + SubAssign,
{
    let numbers = gen_numbers::<T>(num_packets);

    // Construct two empty Quacks.
    let mut acc1 = PowerSumQuack::<T>::new(size);
    let mut acc2 = PowerSumQuack::<T>::new(size);

    // Insert all but num_drop random numbers into the second accumulator.
    for &number in numbers.iter().take(num_packets - num_drop) {
        acc2.insert(number);
    }

    let t1 = Instant::now();
    for &number in numbers.iter().take(num_packets) {
        acc1.insert(number);
    }
    acc1 -= acc2;
    let dropped = acc1.decode_with_log(&numbers);
    let t2 = Instant::now();

    let duration = t2 - t1;
    info!(
        "Decode time (bits = {}, threshold = {}, num_packets={}, \
        false_positives = {}, dropped = {}): {:?}",
        num_bits_id,
        size,
        num_packets,
        dropped.len() - num_drop,
        num_drop,
        duration
    );
    assert!(dropped.len() >= num_drop);
    duration
}

pub fn run_benchmark(
    quack_ty: QuackType,
    num_trials: usize,
    num_packets: usize,
    num_drop: usize,
    params: QuackParams,
) {
    // Allocate buffer for benchmark durations.
    let mut durations: Vec<Duration> = vec![];

    for i in 0..(num_trials + 1) {
        let duration = match quack_ty {
            QuackType::Strawman1a => benchmark_decode_strawman1a(num_packets, num_drop),
            QuackType::Strawman1b => unimplemented!(),
            QuackType::Strawman2 => benchmark_decode_strawman2(num_packets, num_drop),
            QuackType::PowerSum => {
                if params.factor {
                    match params.num_bits_id {
                        16 => todo!(),
                        32 => benchmark_decode_power_sum_factor_u32(
                            params.threshold,
                            num_packets,
                            num_drop,
                        ),
                        64 => todo!(),
                        _ => unimplemented!(),
                    }
                } else if params.precompute {
                    match params.num_bits_id {
                        16 => benchmark_decode_power_sum_precompute_u16(
                            params.threshold,
                            num_packets,
                            num_drop,
                        ),
                        32 => todo!(),
                        64 => todo!(),
                        _ => unimplemented!(),
                    }
                } else if params.montgomery {
                    match params.num_bits_id {
                        16 => unimplemented!(),
                        32 => unimplemented!(),
                        64 => benchmark_decode_power_sum_montgomery_u64(
                            params.threshold,
                            num_packets,
                            num_drop,
                        ),
                        _ => unimplemented!(),
                    }
                } else {
                    match params.num_bits_id {
                        16 => benchmark_decode_power_sum::<u16>(
                            params.threshold,
                            params.num_bits_id,
                            num_packets,
                            num_drop,
                        ),
                        32 => benchmark_decode_power_sum::<u32>(
                            params.threshold,
                            params.num_bits_id,
                            num_packets,
                            num_drop,
                        ),
                        64 => benchmark_decode_power_sum::<u64>(
                            params.threshold,
                            params.num_bits_id,
                            num_packets,
                            num_drop,
                        ),
                        _ => unimplemented!(),
                    }
                }
            }
        };
        if i > 0 {
            durations.push(duration);
        }
    }
    print_summary(durations, num_packets);
}

fn main() {
    env_logger::init();

    let args = Cli::parse();
    debug!("args = {:?}", args);
    run_benchmark(
        args.quack_ty,
        args.num_trials,
        args.num_packets,
        args.num_drop,
        args.quack,
    );
}