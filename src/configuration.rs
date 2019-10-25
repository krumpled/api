extern crate serde;

use serde::Deserialize;
use std::env::var_os;
use std::fs::read;
use std::io::{Error, ErrorKind};
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize)]
pub struct Configuration {
  #[serde(default)]
  pub google: GoogleCredentials,

  #[serde(default)]
  pub krumi: KrumiConfiguration,

  #[serde(default)]
  pub addr: String,
}

impl Default for Configuration {
  fn default() -> Self {
    let google = GoogleCredentials::default();
    let krumi = KrumiConfiguration::default();
    Configuration {
      google,
      krumi,
      addr: String::from("0.0.0.0:8080"),
    }
  }
}

impl FromStr for Configuration {
  type Err = Error;

  fn from_str(source: &str) -> Result<Self, Self::Err> {
    let result = serde_json::from_str::<Configuration>(
      String::from_utf8(read(source)?)
        .or(Err(Error::from(ErrorKind::InvalidData)))?
        .as_str(),
    );

    if let Err(e) = &result {
      println!("[warning] unable to parse '{}': {:?}", source, e);
    }

    result.or(Err(Error::from(ErrorKind::InvalidData)))
  }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GoogleCredentials {
  #[serde(default)]
  pub client_id: String,

  #[serde(default)]
  pub client_secret: String,

  #[serde(default)]
  pub redirect_uri: String,
}

impl Default for GoogleCredentials {
  fn default() -> Self {
    let client_id = var_os("GOOGLE_CLIENT_ID")
      .unwrap_or_default()
      .into_string()
      .unwrap_or_default();
    let client_secret = var_os("GOOGLE_CLIENT_SECRET")
      .unwrap_or_default()
      .into_string()
      .unwrap_or_default();
    let redirect_uri = var_os("GOOGLE_CLIENT_REDIRECT_URI")
      .unwrap_or_default()
      .into_string()
      .unwrap_or_default();

    Self::new(client_id, client_secret, redirect_uri)
  }
}

impl GoogleCredentials {
  pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Self {
    GoogleCredentials {
      client_id,
      client_secret,
      redirect_uri,
    }
  }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct KrumiConfiguration {
  #[serde(default)]
  pub auth_uri: String,
}