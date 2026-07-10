use serde_json::{Value, json};

use super::*;

#[test]
fn parses_a_successful_receipt() {
    let value = json!({
        "status": "0x1",
        "blockNumber": "0x2a",
        "gasUsed": "0x5208",
        "transactionHash": format!("0x{}", "aa".repeat(32)),
    });
    let receipt = parse_receipt(&value, &format!("0x{}", "aa".repeat(32)))
        .unwrap()
        .expect("valid receipt");
    assert!(receipt.status_success);
    assert_eq!(receipt.block_number, 42);
    assert_eq!(receipt.gas_used_hex, "0x5208");
}

#[test]
fn parses_a_reverted_receipt() {
    let value = json!({
        "status": "0x0",
        "blockNumber": "0x2a",
        "gasUsed": "0x5208",
        "transactionHash": format!("0x{}", "bb".repeat(32)),
    });
    let receipt = parse_receipt(&value, &format!("0x{}", "bb".repeat(32)))
        .unwrap()
        .expect("valid receipt");
    assert!(!receipt.status_success);
}

#[test]
fn null_result_is_no_receipt_yet() {
    assert_eq!(
        parse_receipt(&Value::Null, &format!("0x{}", "aa".repeat(32))).unwrap(),
        None
    );
}

#[test]
fn unrecognized_object_shape_is_treated_as_no_receipt_rather_than_an_error() {
    let value = json!({ "unsupported": "eth_getTransactionReceipt" });
    assert_eq!(
        parse_receipt(&value, &format!("0x{}", "aa".repeat(32))).unwrap(),
        None
    );
}

#[test]
fn receipt_requires_the_requested_transaction_identity() {
    let missing = json!({
        "status": "0x1",
        "blockNumber": "0x2a",
        "gasUsed": "0x5208",
    });
    assert!(parse_receipt(&missing, &format!("0x{}", "aa".repeat(32))).is_err());

    let mismatch = json!({
        "status": "0x1",
        "blockNumber": "0x2a",
        "gasUsed": "0x5208",
        "transactionHash": format!("0x{}", "bb".repeat(32)),
    });
    assert!(parse_receipt(&mismatch, &format!("0x{}", "aa".repeat(32))).is_err());
}
