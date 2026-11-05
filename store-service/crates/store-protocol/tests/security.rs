use store_protocol::{
    crypto::{self, Channel},
    upload::{self, UploadClaims, UploadInput},
    Platform,
};

fn pair() -> (Channel, Channel) {
    let key = "11".repeat(32);
    let mut client = crypto::handshake(&key, "account-a", true).unwrap();
    let mut server = crypto::handshake(&key, "account-a", false).unwrap();
    let mut message = [0; crypto::HANDSHAKE_BYTES];
    let mut plain = [0; crypto::HANDSHAKE_BYTES];
    let size = client.write_message(b"registration", &mut message).unwrap();
    let read = server.read_message(&message[..size], &mut plain).unwrap();
    assert_eq!(&plain[..read], b"registration");
    let size = server.write_message(&[], &mut message).unwrap();
    client.read_message(&message[..size], &mut plain).unwrap();
    (Channel::new(client).unwrap(), Channel::new(server).unwrap())
}

#[test]
fn encrypted_json_fragments_and_round_trips_in_both_directions() {
    let (mut client, mut server) = pair();
    let payload = vec![b'x'; 200_000];
    let records = client.encrypt(&payload).unwrap();
    assert!(records.len() > 1);
    let mut result = None;
    for record in records {
        assert!(record.len() <= crypto::MAX_RECORD_BYTES);
        result = server.decrypt(&record).unwrap();
    }
    assert_eq!(result.unwrap(), payload);
    let record = server.encrypt(b"result").unwrap().remove(0);
    assert_eq!(client.decrypt(&record).unwrap().unwrap(), b"result");
    assert!(client
        .encrypt(&vec![0; crypto::MAX_JSON_BYTES + 1])
        .is_err());
}

#[test]
fn wrong_keys_tampering_replay_and_cross_session_replay_are_rejected() {
    let (mut client, mut server) = pair();
    let record = client.encrypt(b"sensitive command").unwrap().remove(0);
    assert!(!record
        .windows(b"sensitive".len())
        .any(|v| v == b"sensitive"));
    assert!(server.decrypt(&record).unwrap().is_some());
    assert!(server.decrypt(&record).is_err());
    let (_, mut new_server) = pair();
    assert!(new_server.decrypt(&record).is_err());
    let (mut client, mut server) = pair();
    let mut altered = client.encrypt(b"command").unwrap().remove(0);
    altered[0] ^= 1;
    assert!(server.decrypt(&altered).is_err());
    let (mut client, mut server) = pair();
    let records = client.encrypt(&vec![1; 100_000]).unwrap();
    assert!(server.decrypt(&records[1]).is_err());
    let mut client = crypto::handshake(&"11".repeat(32), "account-a", true).unwrap();
    let mut server = crypto::handshake(&"22".repeat(32), "account-a", false).unwrap();
    let mut message = [0; crypto::HANDSHAKE_BYTES];
    let mut plain = [0; crypto::HANDSHAKE_BYTES];
    let size = client.write_message(b"auth", &mut message).unwrap();
    assert!(server.read_message(&message[..size], &mut plain).is_err());
}

#[test]
fn credentials_bind_identity_expiration_and_file_constraints() {
    let time = upload::now().unwrap();
    let mut claims = UploadClaims {
        iss: "testflying-store-api".into(),
        aud: "connector:node-a".into(),
        exp: time + 1800,
        iat: time,
        asset_id: "00000000-0000-4000-8000-000000000001".into(),
        connector_id: "node-a".into(),
        platform: Platform::Ios,
        bundle_identifier: "com.example.app".into(),
        file: UploadInput {
            file_name: "screen.png".into(),
            content_type: "image/png".into(),
            size_bytes: 3_000_000,
            sha256: "11".repeat(32),
        },
    };
    let key = "22".repeat(32);
    let token = upload::sign(&claims, &key).unwrap();
    assert!(upload::verify(&token, &key, "node-a").is_ok());
    assert!(upload::verify(&token, &key, "node-b").is_err());
    assert!(upload::verify(&token, &"33".repeat(32), "node-a").is_err());
    claims.exp = time + 1801;
    assert!(upload::verify(&upload::sign(&claims, &key).unwrap(), &key, "node-a").is_err());
    claims.iat = time - 1200;
    claims.exp = time + 600;
    assert!(upload::verify(&upload::sign(&claims, &key).unwrap(), &key, "node-a").is_ok());
    claims.iat = time;
    claims.exp = time - 1;
    assert!(upload::verify(&upload::sign(&claims, &key).unwrap(), &key, "node-a").is_err());
    claims.exp = time + 1800;
    claims.file.size_bytes = upload::MAX_FILE_BYTES + 1;
    assert!(upload::verify(&upload::sign(&claims, &key).unwrap(), &key, "node-a").is_err());
    for invalid in [
        "file:///tmp/key",
        "http://u:p@host",
        "http://host?token=x",
        "https://host/#x",
        " https://host",
    ] {
        assert!(upload::normalize_base_url(invalid).is_err());
    }
    assert_eq!(
        upload::normalize_base_url("https://node.example/path/").unwrap(),
        "https://node.example/path"
    );
}
