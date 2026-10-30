use nekochat_search::Money;
use serde_json::json;
#[test]
fn usage_money_accepts_only_nonnegative_supported_currency_amounts() {
    assert!(serde_json::from_value::<Money>(json!({"amount":-1,"currency":"USD"})).is_err());
    assert!(serde_json::from_value::<Money>(json!({"amount":1,"currency":"EUR"})).is_err());
    let amount: Money = serde_json::from_value(json!({"amount":0.005,"currency":"CNY"})).unwrap();
    assert_eq!(amount.amount,0.005);
    assert_eq!(amount.currency,"CNY");
}
