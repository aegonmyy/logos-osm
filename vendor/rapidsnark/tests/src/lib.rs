#[cfg(test)]
mod tests {
    use anyhow::Result;
    use num_bigint::BigInt;
    use std::{collections::HashMap, str::FromStr};

    use rust_rapidsnark::{parse_bigints_to_witness, WtnsFn};

    rust_witness::witness!(multiplier2);
    rust_witness::witness!(keccak256256test);

    fn bytes_to_bits(bytes: &[u8]) -> Vec<bool> {
        let mut bits = Vec::new();
        for &byte in bytes {
            for j in 0..8 {
                let bit = (byte >> j) & 1;
                bits.push(bit == 1);
            }
        }
        bits
    }

    fn bytes_to_circuit_inputs(input_vec: &[u8]) -> HashMap<String, Vec<String>> {
        let bits = bytes_to_bits(input_vec);
        let converted_vec: Vec<String> = bits
            .into_iter()
            .map(|bit| (bit as i32).to_string())
            .collect();
        let mut inputs = HashMap::new();
        inputs.insert("in".to_string(), converted_vec);
        inputs
    }

    fn compute_witness(
        inputs: HashMap<String, Vec<String>>,
        witness_fn: WtnsFn,
    ) -> Result<Vec<u8>> {
        // Form the inputs
        let bigint_inputs = inputs
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v.into_iter()
                        .map(|i| BigInt::from_str(&i).unwrap())
                        .collect(),
                )
            })
            .collect();

        let wtns: Vec<BigInt> = witness_fn(bigint_inputs);
        let witnesscalc_wtns = parse_bigints_to_witness(wtns)?;
        Ok(witnesscalc_wtns)
    }

    #[test]
    fn test_prove_rapidsnark() -> Result<()> {
        // Create a new MoproCircom instance
        let zkey_path = "./test-vectors/multiplier2_final.zkey";

        let mut inputs = HashMap::new();
        let a = BigInt::from_str(
            "21888242871839275222246405745257275088548364400416034343698204186575808495616",
        )
        .unwrap();
        let b = BigInt::from(1u8);
        inputs.insert("a".to_string(), vec![a.to_string()]);
        inputs.insert("b".to_string(), vec![b.to_string()]);

        // Generate Witness Buffer
        let wtns_buffer = compute_witness(inputs, multiplier2_witness)?;

        // Generate Proof
        let proof_result =
            rust_rapidsnark::groth16_prover_zkey_file_wrapper(zkey_path, wtns_buffer)?;

        let vkey = std::fs::read_to_string("./test-vectors/multiplier2.vkey.json")?;
        let valid = rust_rapidsnark::groth16_verify_wrapper(
            &proof_result.proof,
            &proof_result.public_signals,
            &vkey,
        )?;
        assert!(valid);
        Ok(())
    }

    #[test]
    fn test_prove_rapidsnark_zkey_buffer() -> Result<()> {
        let zkey_path = "./test-vectors/multiplier2_final.zkey";
        let zkey_buffer = std::fs::read(zkey_path)?;

        let mut inputs = HashMap::new();
        inputs.insert("a".to_string(), vec!["3".to_string()]);
        inputs.insert("b".to_string(), vec!["11".to_string()]);

        let wtns_buffer = compute_witness(inputs, multiplier2_witness)?;
        let path_proof_result =
            rust_rapidsnark::groth16_prover_zkey_file_wrapper(zkey_path, wtns_buffer.clone())?;
        let buffer_proof_result =
            rust_rapidsnark::groth16_prover_zkey_buffer_wrapper(&zkey_buffer, &wtns_buffer)?;

        assert_eq!(
            path_proof_result.public_signals,
            buffer_proof_result.public_signals
        );

        let vkey = std::fs::read_to_string("./test-vectors/multiplier2.vkey.json")?;
        let path_valid = rust_rapidsnark::groth16_verify_wrapper(
            &path_proof_result.proof,
            &path_proof_result.public_signals,
            &vkey,
        )?;
        let buffer_valid = rust_rapidsnark::groth16_verify_wrapper(
            &buffer_proof_result.proof,
            &buffer_proof_result.public_signals,
            &vkey,
        )?;
        assert!(path_valid);
        assert!(buffer_valid);
        Ok(())
    }

    #[test]
    fn test_prove_rapidsnark_keccak() -> Result<()> {
        // Create a new MoproCircom instance
        let zkey_path = "./test-vectors/keccak256_256_test_final.zkey";
        // Prepare inputs
        let input_vec = vec![
            116, 101, 115, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0,
        ];

        let inputs = bytes_to_circuit_inputs(&input_vec);

        // Generate Witness Buffer
        let wtns_buffer = compute_witness(inputs, keccak256256test_witness)?;
        let wtns_data = std::fs::read("./test-vectors/keccak256_256_test.wtns")?;
        assert_eq!(wtns_buffer, wtns_data);

        // Generate Proof
        let proof_result =
            rust_rapidsnark::groth16_prover_zkey_file_wrapper(zkey_path, wtns_buffer)?;

        let vkey = std::fs::read_to_string("./test-vectors/keccak256_256_test.vkey.json")?;
        let valid = rust_rapidsnark::groth16_verify_wrapper(
            &proof_result.proof,
            &proof_result.public_signals,
            &vkey,
        )?;
        assert!(valid);
        Ok(())
    }

    /// Regression guard for the rapidsnark concurrent-proving deadlock.
    ///
    /// rapidsnark before v0.0.8 shared a single global C++ thread pool with no
    /// pool-level lock, so concurrent provers could strand each other on a
    /// worker and hang. The hang was non-deterministic (more likely the more
    /// proofs ran at once); v0.0.8 serializes the pool and fixes it.
    ///
    /// This runs several proofs at once with a timeout watchdog, so a
    /// reintroduced deadlock fails the test instead of hanging CI forever. It is
    /// a best-effort, timing-based check: a failure here almost certainly means
    /// the deadlock is back (e.g. an ffiasm downgrade), not random flakiness.
    #[test]
    fn concurrent_proving_does_not_deadlock() -> Result<()> {
        use std::{sync::mpsc, time::Duration};

        const CONCURRENCY: usize = 8;
        const TIMEOUT: Duration = Duration::from_secs(60);

        let zkey_path = "./test-vectors/multiplier2_final.zkey";

        let mut inputs = HashMap::new();
        inputs.insert("a".to_string(), vec!["3".to_string()]);
        inputs.insert("b".to_string(), vec!["11".to_string()]);
        let wtns_buffer = compute_witness(inputs, multiplier2_witness)?;

        let (tx, rx) = mpsc::channel();
        for _ in 0..CONCURRENCY {
            let wtns = wtns_buffer.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                rust_rapidsnark::groth16_prover_zkey_file_wrapper(zkey_path, wtns).unwrap();
                let _ = tx.send(());
            });
        }
        drop(tx);

        for completed in 0..CONCURRENCY {
            rx.recv_timeout(TIMEOUT).unwrap_or_else(|_| {
                panic!(
                    "concurrent proving deadlocked: only {completed}/{CONCURRENCY} \
                     proofs finished within {TIMEOUT:?}"
                )
            });
        }
        Ok(())
    }
}
