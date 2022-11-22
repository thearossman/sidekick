use std::fmt;

use crate::{Quack, Identifier};
use crate::arithmetic::*;

pub type IdentifierLog = Vec<Identifier>;

pub struct DecodedQuack<'a> {
    quack: &'a Quack,
    log: &'a IdentifierLog,
    // Indexes of the missing packets in the identifier log.
    indexes: Vec<usize>,
}

impl<'a> fmt::Display for DecodedQuack<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.indexes)
    }
}

impl<'a> fmt::Debug for DecodedQuack<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("DecodedQuack")
         .field("quack_count", &self.quack.count)
         .field("log_length", &self.log.len())
         .field("indexes", &self.indexes)
         .finish()
    }
}

impl<'a> DecodedQuack<'a> {
    pub fn decode(quack: &'a Quack, log: &'a IdentifierLog) -> Self {
        let num_packets = log.len();
        let num_missing = quack.count;
        let coeffs = {
            let mut coeffs = (0..num_missing)
                .map(|_| ModularInteger::zero())
                .collect();
            quack.to_polynomial_coefficients(&mut coeffs);
            coeffs
        };
        let indexes = (0..num_packets)
            .filter(|&i| {
                MonicPolynomialEvaluator::eval(&coeffs, log[i]).is_zero()
            })
            .collect();
        Self {
            quack,
            log,
            indexes,
        }
    }

    /// The number of consecutive missing packets at the end of the identifier
    /// log. These packets were likely in transit when the quACK was sent.
    pub fn num_suffix(&self) -> usize {
        if self.indexes.is_empty() {
            0
        } else {
            let mut last = self.log.len() - 1;
            let mut count = 0;
            let mut i = self.indexes.len();
            while i > 0 {
                if self.indexes[i - 1] == last {
                    last -= 1;
                    count += 1;
                    i -= 1;
                } else {
                    break;
                }
            }
            count
        }
    }

    /// The number of missing packets outside of the suffix of missing packets.
    /// It is more likely that these were dropped.
    pub fn num_missing(&self) -> usize {
        self.total_num_missing() - self.num_suffix()
    }

    /// The total number of missing packets = num_suffix() + num_missing().
    pub fn total_num_missing(&self) -> usize {
        self.indexes.len()
    }

    /// The indexes of the missing packets outside of the suffix of the missing
    /// packets. Note that the total number of missing of packets plus the
    /// number of missing packets in the suffix may exceed the count in the
    /// quACK due to false positives.
    pub fn missing(&self) -> &[usize] {
        &self.indexes[..(self.total_num_missing() - self.num_suffix())]
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_quack_decode() {
        let log = vec![1, 2, 3, 4, 5, 6];
        let mut q1 = Quack::new(3);
        for x in &log {
            q1.insert(*x);
        }
        let mut q2 = Quack::new(3);
        q2.insert(1);
        q2.insert(3);
        q2.insert(4);

        // Check the result
        let quack = q1 - q2;
        let result = DecodedQuack::decode(&quack, &log);
        assert_eq!(result.num_suffix(), 2);
        assert_eq!(result.num_missing(), 1);
        assert_eq!(result.total_num_missing(), 3);
        assert_eq!(result.missing(), &vec![1]);
    }
}