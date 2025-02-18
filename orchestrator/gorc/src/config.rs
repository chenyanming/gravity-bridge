use aws_sdk_secretsmanager::client::Client;
use cosmos_gravity::crypto::DEFAULT_HD_PATH;
use serde::{Deserialize, Serialize};
use signatory::FsKeyStore;
use std::io;
use std::net::SocketAddr;
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Keystore {
    File(String),
    Aws,
}

impl Default for Keystore {
    fn default() -> Self {
        Keystore::File("/tmp/keystore".to_owned())
    }
}

async fn get_secret(
    secret_id: String,
) -> Result<pkcs8::PrivateKeyDocument, aws_sdk_secretsmanager::Error> {
    let shared_config = aws_config::load_from_env().await;

    let client = Client::new(&shared_config);
    let req = client.get_secret_value().secret_id(secret_id);
    let resp = req.send().await?;
    let e1 = aws_sdk_secretsmanager::Error::Unhandled(Box::<io::Error>::new(
        io::ErrorKind::NotFound.into(),
    ));
    let secret = resp.secret_string.ok_or(e1)?;
    let e2 = aws_sdk_secretsmanager::Error::Unhandled(Box::<io::Error>::new(
        io::ErrorKind::Other.into(),
    ));
    pkcs8::PrivateKeyDocument::from_pem(&secret).map_err(|_| e2)
}

async fn set_secret(
    secret_id: String,
    secret: &pkcs8::PrivateKeyDocument,
) -> Result<(), aws_sdk_secretsmanager::Error> {
    let shared_config = aws_config::load_from_env().await;

    let client = Client::new(&shared_config);
    let req = client
        .create_secret()
        .name(secret_id)
        .secret_string(secret.to_pem().as_str());
    let _ = req.send().await?;
    Ok(())
}

async fn delete_secret(secret_id: String) -> Result<(), aws_sdk_secretsmanager::Error> {
    let shared_config = aws_config::load_from_env().await;

    let client = Client::new(&shared_config);
    let req = client.delete_secret().secret_id(secret_id);
    let _ = req.send().await?;
    Ok(())
}

async fn describe_secret(
    secret_id: String,
) -> Result<signatory::KeyInfo, aws_sdk_secretsmanager::Error> {
    let shared_config = aws_config::load_from_env().await;

    let client = Client::new(&shared_config);
    let e = aws_sdk_secretsmanager::Error::Unhandled(Box::<io::Error>::new(
        io::ErrorKind::Other.into(),
    ));
    let req = client.describe_secret().secret_id(secret_id);
    let r = req.send().await?;
    if let Some(name) = r.name {
        Ok(signatory::KeyInfo {
            name: signatory::KeyName::new(name).map_err(|_| e)?,
            algorithm: None,
            encrypted: false,
        })
    } else {
        Err(e)
    }
}

impl Keystore {
    /// Load a PKCS#8 key from the keystore.
    pub fn load(&self, name: &signatory::KeyName) -> signatory::Result<pkcs8::PrivateKeyDocument> {
        match self {
            Keystore::File(path) => {
                let keystore = Path::new(path);
                let keystore = FsKeyStore::create_or_open(keystore)?;
                keystore.load(&name)
            }
            Keystore::Aws => {
                let rt = tokio::runtime::Runtime::new()?;

                let key = rt.block_on(get_secret(name.to_string()));

                key.map_err(|e| signatory::Error::Io(io::Error::new(io::ErrorKind::Other, e)))
            }
        }
    }
    /// Get information about a key with the given name.
    pub fn info(&self, name: &signatory::KeyName) -> signatory::Result<signatory::KeyInfo> {
        match self {
            Keystore::File(path) => {
                let keystore = Path::new(path);
                let keystore = FsKeyStore::create_or_open(keystore)?;
                keystore.info(&name)
            }
            Keystore::Aws => {
                let rt = tokio::runtime::Runtime::new()?;
                let info = rt.block_on(describe_secret(name.to_string()));
                info.map_err(|e| signatory::Error::Io(io::Error::new(io::ErrorKind::Other, e)))
            }
        }
    }

    /// Import a PKCS#8 key into the keystore.
    pub fn store(
        &self,
        name: &signatory::KeyName,
        der: &pkcs8::PrivateKeyDocument,
    ) -> signatory::Result<()> {
        match self {
            Keystore::File(path) => {
                let keystore = Path::new(path);
                let keystore = FsKeyStore::create_or_open(keystore)?;
                keystore.store(&name, der)
            }
            Keystore::Aws => {
                let rt = tokio::runtime::Runtime::new()?;

                rt.block_on(set_secret(name.to_string(), der))
                    .map_err(|e| signatory::Error::Io(io::Error::new(io::ErrorKind::Other, e)))
            }
        }
    }

    /// Delete a PKCS#8 key from the keystore.
    pub fn delete(&self, name: &signatory::KeyName) -> signatory::Result<()> {
        match self {
            Keystore::File(path) => {
                let keystore = Path::new(path);
                let keystore = FsKeyStore::create_or_open(keystore)?;
                keystore.delete(&name)
            }
            Keystore::Aws => {
                let rt = tokio::runtime::Runtime::new()?;

                rt.block_on(delete_secret(name.to_string()))
                    .map_err(|e| signatory::Error::Io(io::Error::new(io::ErrorKind::Other, e)))
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GorcConfig {
    pub keystore: Keystore,
    pub gravity: GravitySection,
    pub ethereum: EthereumSection,
    pub cosmos: CosmosSection,
    pub metrics: MetricsSection,
}

impl GorcConfig {
    fn load_secret_key(&self, name: String) -> k256::elliptic_curve::SecretKey<k256::Secp256k1> {
        let name = name.parse().expect("Could not parse name");
        let key = self.keystore.load(&name).expect("Could not load key");
        return key.to_pem().parse().expect("Could not parse pem");
    }

    pub fn load_clarity_key(&self, name: String) -> clarity::PrivateKey {
        let key = self.load_secret_key(name).to_bytes();
        return clarity::PrivateKey::from_slice(&key).expect("Could not convert key");
    }

    pub fn load_deep_space_key(&self, name: String) -> cosmos_gravity::crypto::PrivateKey {
        let key = self.load_secret_key(name).to_bytes();
        let key = deep_space::utils::bytes_to_hex_str(&key);
        return key.parse().expect("Could not parse private key");
    }
}

impl Default for GorcConfig {
    fn default() -> Self {
        Self {
            keystore: Keystore::default(),
            gravity: GravitySection::default(),
            ethereum: EthereumSection::default(),
            cosmos: CosmosSection::default(),
            metrics: MetricsSection::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GravitySection {
    pub contract: String,
    pub fees_denom: String,
}

impl Default for GravitySection {
    fn default() -> Self {
        Self {
            contract: "0x0000000000000000000000000000000000000000".to_owned(),
            fees_denom: "stake".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EthereumSection {
    pub key_derivation_path: String,
    pub rpc: String,
    pub gas_price_multiplier: f32,
    pub blocks_to_search: u64,
}

impl Default for EthereumSection {
    fn default() -> Self {
        Self {
            key_derivation_path: "m/44'/60'/0'/0/0".to_owned(),
            rpc: "http://localhost:8545".to_owned(),
            gas_price_multiplier: 1.0f32,
            blocks_to_search: 5000,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CosmosSection {
    pub key_derivation_path: String,
    pub grpc: String,
    pub prefix: String,
    pub gas_price: GasPrice,
}

impl Default for CosmosSection {
    fn default() -> Self {
        Self {
            key_derivation_path: DEFAULT_HD_PATH.to_owned(),
            grpc: "http://localhost:9090".to_owned(),
            prefix: "cosmos".to_owned(),
            gas_price: GasPrice::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GasPrice {
    pub amount: f64,
    pub denom: String,
}

impl Default for GasPrice {
    fn default() -> Self {
        Self {
            amount: 0.001,
            denom: "stake".to_owned(),
        }
    }
}

impl GasPrice {
    pub fn as_tuple(&self) -> (f64, String) {
        (self.amount, self.denom.to_owned())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsSection {
    pub listen_addr: SocketAddr,
}

impl Default for MetricsSection {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:3000".parse().unwrap(),
        }
    }
}
