pub mod crypto;
pub mod types;

mod dong_client;
mod error;
mod http;
mod indexer_client;
mod mmn_client;
mod zk_client;

pub use crypto::{
    address_from_user_id, compare_big_decimal, generate_ephemeral_key_pair,
    scale_amount_to_decimals, validate_address, validate_amount,
};
pub use dong_client::DongClient;
pub use error::{MmnError, Result};
pub use indexer_client::{FILTER_ALL, FILTER_RECEIVED, FILTER_SENT, IndexerClient};
pub use mmn_client::MmnClient;
pub use types::*;
pub use zk_client::ZkClient;

#[cfg(test)]
mod tests {
    use super::crypto;
    use super::types::{TX_TYPE_TRANSFER_BY_KEY, TX_TYPE_TRANSFER_BY_ZK, TxMsg};

    const FIXED_PKCS8_HEX: &str = "302e020100300b06032b6570042204200102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    const FIXED_PUBKEY_B58: &str = "9C6hybhQ6Aycep9jaUnP6uL9ZYvDjUp1aSkFWPUFJtpj";

    fn fixed_tx(tx_type: u8) -> TxMsg {
        TxMsg {
            tx_type,
            sender: "SENDER".into(),
            recipient: "RECIP".into(),
            amount: "1000000".into(),
            timestamp: 1_700_000_000_000,
            text_data: String::new(),
            nonce: 5,
            extra_info: String::new(),
            zk_proof: String::new(),
            zk_pub: String::new(),
        }
    }

    #[test]
    fn address_from_user_id_matches_js() {
        assert_eq!(
            crypto::address_from_user_id("user123"),
            "GUvs4AihbbXtcAsbuuH8bXEgYXEtbYdavVgWMHBDAQMT"
        );
        assert_eq!(
            crypto::address_from_user_id("1767478432163172999"),
            "BNktLdutWyfPgNAXZDar1vuhVq8VSCuaDGa3CRVBZnRx"
        );
    }

    #[test]
    fn address_is_stable_and_valid() {
        let a = crypto::address_from_user_id("abc");
        let b = crypto::address_from_user_id("abc");
        assert_eq!(a, b);
        assert!(crypto::validate_address(&a));
        assert!(!crypto::validate_address("not-base58-0OIl"));
        assert!(!crypto::validate_address(""));
    }

    #[test]
    fn serialize_transaction_matches_js_pipe_format() {
        let tx = fixed_tx(TX_TYPE_TRANSFER_BY_KEY);
        assert_eq!(
            crypto::serialize_transaction(&tx),
            b"1|SENDER|RECIP|1000000||5|".to_vec()
        );
    }

    #[test]
    fn sign_transfer_by_key_matches_nacl_vector() {
        let tx = fixed_tx(TX_TYPE_TRANSFER_BY_KEY);
        let signature = crypto::sign_transaction(&tx, FIXED_PKCS8_HEX).unwrap();
        assert_eq!(
            signature,
            "5atLETo5aLdXBHLsPJdCoRb22dY7W6mkwL5mmndxuMjFRkFdhnwmkzt73UdoYJ3odhVm7vpWYcHCHc11tqEmRJCE"
        );
    }

    #[test]
    fn sign_transfer_by_zk_wraps_pubkey_and_sig_like_js() {
        let tx = fixed_tx(TX_TYPE_TRANSFER_BY_ZK);
        let signature = crypto::sign_transaction(&tx, FIXED_PKCS8_HEX).unwrap();
        let decoded = bs58::decode(&signature).into_vec().unwrap();
        let json = String::from_utf8(decoded).unwrap();
        assert_eq!(
            json,
            "{\"PubKey\":\"ebVWLo/mVPlAeLES6KmLp5AfhTrmlb7X4OORC60ElmQ=\",\"Sig\":\"ii39WQ2Zb6YO+yZEkN74kk7hWlxWHLZPbXamqKT2MOFzt/O/W0tA3JIRQ6qNbOrmxu7GfihmQD6G8+tVh/F3BA==\"}"
        );
    }

    #[test]
    fn generated_key_pair_has_js_pkcs8_shape() {
        let pair = crypto::generate_ephemeral_key_pair().unwrap();
        assert_eq!(pair.private_key.len(), 96);
        assert!(
            pair.private_key
                .starts_with("302e020100300b06032b657004220420")
        );
        assert!(crypto::validate_address(&pair.public_key));

        let signed =
            crypto::sign_transaction(&fixed_tx(TX_TYPE_TRANSFER_BY_KEY), &pair.private_key);
        assert!(signed.is_ok());
    }

    #[test]
    fn generated_private_key_derives_encoded_public_key() {
        let signed =
            crypto::sign_transaction(&fixed_tx(TX_TYPE_TRANSFER_BY_ZK), FIXED_PKCS8_HEX).unwrap();
        let decoded = bs58::decode(&signed).into_vec().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        let pub_b64 = json["PubKey"].as_str().unwrap();
        let pub_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, pub_b64).unwrap();
        assert_eq!(bs58::encode(pub_bytes).into_string(), FIXED_PUBKEY_B58);
    }

    #[test]
    fn scale_amount_matches_bigint_multiplication() {
        assert_eq!(
            crypto::scale_amount_to_decimals("100", 6).unwrap(),
            "100000000"
        );
        assert_eq!(crypto::scale_amount_to_decimals("1", 6).unwrap(), "1000000");
        assert_eq!(crypto::scale_amount_to_decimals("0", 6).unwrap(), "0");
        assert!(crypto::scale_amount_to_decimals("abc", 6).is_err());
    }

    #[test]
    fn validate_amount_compares_scaled_values() {
        assert!(crypto::validate_amount("1000000", "1000000"));
        assert!(crypto::validate_amount("2000000", "1000000"));
        assert!(!crypto::validate_amount("500000", "1000000"));
        assert!(crypto::validate_amount(
            "340282366920938463463374607431768211456",
            "1000000"
        ));
    }

    #[test]
    fn signed_tx_serializes_to_addtx_wire_shape() {
        let tx = fixed_tx(TX_TYPE_TRANSFER_BY_ZK);
        let signed = super::types::SignedTx {
            tx_msg: tx,
            signature: "sig".into(),
        };
        let value = serde_json::to_value(&signed).unwrap();
        assert_eq!(value["tx_msg"]["type"], 0);
        assert_eq!(value["tx_msg"]["sender"], "SENDER");
        assert_eq!(value["tx_msg"]["text_data"], "");
        assert_eq!(value["tx_msg"]["zk_pub"], "");
        assert_eq!(value["signature"], "sig");
    }

    #[test]
    fn extra_info_transfer_token_matches_react_fields() {
        let extra = super::types::ExtraInfo {
            transfer_type: super::types::TRANSFER_TYPE_TRANSFER_TOKEN.to_string(),
            user_receiver_id: Some("r".into()),
            user_sender_id: Some("s".into()),
            user_sender_username: Some("u".into()),
            extra_attribute: Some(String::new()),
            ..Default::default()
        };
        let json = serde_json::to_string(&extra).unwrap();
        assert_eq!(
            json,
            r#"{"type":"transfer_token","UserReceiverId":"r","UserSenderId":"s","UserSenderUsername":"u","ExtraAttribute":""}"#
        );
    }

    #[test]
    fn extra_info_give_coffee_matches_react_fields() {
        let extra = super::types::ExtraInfo {
            transfer_type: super::types::TRANSFER_TYPE_GIVE_COFFEE.to_string(),
            channel_id: Some("0".into()),
            clan_id: Some("0".into()),
            message_ref_id: Some("1".into()),
            user_receiver_id: Some("r".into()),
            user_sender_id: Some("s".into()),
            user_sender_username: Some("u".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&extra).unwrap();
        assert_eq!(
            json,
            r#"{"type":"give_coffee","ChannelId":"0","ClanId":"0","MessageRefId":"1","UserReceiverId":"r","UserSenderId":"s","UserSenderUsername":"u"}"#
        );
    }

    #[test]
    fn mmn_constants_match_react() {
        assert_eq!(super::types::TRANSFER_TYPE_GIVE_COFFEE, "give_coffee");
        assert_eq!(super::types::TRANSFER_TYPE_TRANSFER_TOKEN, "transfer_token");
        assert_eq!(super::types::TRANSFER_TYPE_UNLOCK_ITEM, "unlock_item");
        assert_eq!(super::types::DECIMALS, 6);
    }
}
