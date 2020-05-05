use http::response::{Builder, Response};
use http::status::StatusCode;
use http::Uri;
use http::{Method, Request};
use log::info;
use r2d2::PooledConnection;
use r2d2_postgres::PostgresConnectionManager as Postgres;
use serde::{Deserialize, Serialize};
use std::io::{Error, ErrorKind};
use url::form_urlencoded;

use crate::authorization::AuthorizationUrls;
use crate::configuration::GoogleCredentials;
use crate::constants;
use crate::persistence::RecordStore;
use crate::session::SessionStore;

const FIND_USER: &'static str = include_str!("data-store/find_user.sql");
const CREATE_USER: &'static str = include_str!("data-store/create_user.sql");

#[derive(Debug, PartialEq, Deserialize)]
struct TokenExchangePayload {
  access_token: String,
}

#[derive(Debug, Clone, Deserialize, Default, Serialize)]
struct UserInfoPayload {
  name: String,
  sub: String,
  email: String,
  picture: String,
}

// Given the token returned from an oauth code exchange, load the user's information from the
// google api.
async fn fetch_info(authorization: TokenExchangePayload) -> Result<UserInfoPayload, Error> {
  let client = isahc::HttpClient::new().map_err(|e| Error::new(ErrorKind::Other, e))?;
  let mut request = Request::builder();
  let bearer = format!("Bearer {}", authorization.access_token);
  request
    .method(Method::GET)
    .uri(constants::google_info_url())
    .header("Authorization", bearer.as_str());

  match client.send(
    request
      .body(())
      .map_err(|e| Error::new(ErrorKind::Other, e))?,
  ) {
    Ok(mut response) if response.status() == 200 => {
      serde_json::from_reader(response.body_mut()).map_err(|e| Error::new(ErrorKind::Other, e))
    }
    Ok(response) => Err(Error::new(
      ErrorKind::Other,
      format!("bad response satus from google sso: {}", response.status()),
    )),
    Err(e) => Err(Error::new(ErrorKind::Other, format!("{}", e))),
  }
}

// Given an oauth code returned from google, attempt to exchange the code for a real auth token
// that will provide access to user information.
async fn exchange_code(
  code: &str,
  authorization: &AuthorizationUrls,
) -> Result<TokenExchangePayload, Error> {
  let client = isahc::HttpClient::new().map_err(|e| Error::new(ErrorKind::Other, e))?;
  let (
    exchange_url,
    GoogleCredentials {
      client_id,
      client_secret,
      redirect_uri,
    },
  ) = &authorization.exchange;

  let encoded: String = form_urlencoded::Serializer::new(String::new())
    .append_pair("code", code)
    .append_pair("client_id", client_id.as_str())
    .append_pair("client_secret", client_secret.as_str())
    .append_pair("redirect_uri", redirect_uri.as_str())
    .append_pair("grant_type", "authorization_code")
    .finish();

  match client.post(exchange_url, encoded) {
    Ok(mut response) if response.status() == StatusCode::OK => {
      let body = response.body_mut();
      let payload = match serde_json::from_reader(body) {
        Ok(p) => p,
        Err(e) => {
          return Err(Error::new(
            ErrorKind::Other,
            format!("unable to parse response body: {:?}", e),
          ));
        }
      };
      Ok(payload)
    }
    Ok(response) => Err(Error::new(
      ErrorKind::Other,
      format!("bad response from google sso: {:?}", response.status()),
    )),
    Err(e) => Err(Error::new(
      ErrorKind::Other,
      format!("unable to send code to google sso: {:?}", e),
    )),
  }
}

// Given user information loaded from the api, attempt to save the information into the persistence
// engine.
fn make_user(
  details: &UserInfoPayload,
  conn: &PooledConnection<Postgres>,
) -> Result<String, Error> {
  let UserInfoPayload {
    email,
    name,
    sub,
    picture: _,
  } = details;
  conn
    .execute(CREATE_USER, &[&email, &name, &email, &name, &sub])
    .map_err(|e| Error::new(ErrorKind::Other, e))?;

  let tenant = conn.query(FIND_USER, &[&sub])?;
  match tenant.iter().nth(0) {
    Some(row) => match row.get_opt::<_, String>(0) {
      Some(Ok(id)) => Ok(id),
      _ => Err(Error::new(
        ErrorKind::Other,
        "Found matching row, but unable to parse",
      )),
    },
    _ => Err(Error::new(
      ErrorKind::Other,
      "Unable to find previously inserted user",
    )),
  }
}

// Attempt to find a user based on the google account id returned. If none is found, attempt to
// find by the email address and make sure to backfill the google account. If there is still no
// matching user information, attempt to create a new user and google account.
fn find_or_create_user(profile: &UserInfoPayload, records: &RecordStore) -> Result<String, Error> {
  let conn = records.get()?;
  info!("loaded user info: {:?}", profile);

  let tenant = conn.query(FIND_USER, &[&profile.sub])?;

  match tenant.iter().nth(0) {
    Some(row) => match row.get_opt::<_, String>(0) {
      Some(Ok(id)) => {
        info!("found existing user {}", id);
        Ok(id)
      }
      _ => Err(Error::new(
        ErrorKind::Other,
        "Unable to parse a valid id from matching row",
      )),
    },
    None => {
      info!("no matching user, creating");
      make_user(&profile, &conn)
    }
  }
}

// This is the route handler that is used as the redirect uri of the google client. It is
// responsible for receiving the code from the successful oauth prompt and redirecting the user to
// the krumpled ui.
pub async fn callback(
  uri: Uri,
  session: &SessionStore,
  records: &RecordStore,
  authorization: &AuthorizationUrls,
) -> Result<Response<Option<u8>>, Error> {
  let query = uri.query().unwrap_or_default().as_bytes();
  let code = match form_urlencoded::parse(query).find(|(key, _)| key == "code") {
    Some((_, code)) => code,
    None => {
      return Builder::new()
        .status(404)
        .body(None)
        .map_err(|e| Error::new(ErrorKind::Other, e))
    }
  };

  let payload = match exchange_code(&code, authorization).await {
    Ok(payload) => payload,
    Err(e) => {
      info!("[warning] unable ot exchange code: {}", e);
      return Builder::new()
        .status(404)
        .body(None)
        .map_err(|e| Error::new(ErrorKind::Other, e));
    }
  };

  let profile = match fetch_info(payload).await {
    Ok(info) => info,
    Err(e) => {
      info!("[warning] unable to fetch user info: {}", e);
      return Builder::new()
        .status(404)
        .body(None)
        .map_err(|e| Error::new(ErrorKind::Other, e));
    }
  };

  let uid = match find_or_create_user(&profile, records) {
    Ok(id) => id,
    Err(e) => {
      info!("[warning] unable to create/find user: {:?}", e);
      return Builder::new()
        .status(404)
        .body(None)
        .map_err(|e| Error::new(ErrorKind::Other, e));
    }
  };

  let token = session.create(&uid).await?;
  info!("creating session for user id: {}", token);

  Builder::new()
    .status(301)
    .header("Location", authorization.callback.clone())
    .body(None)
    .map_err(|e| Error::new(ErrorKind::Other, e))
}

pub async fn identify() -> Result<Response<Option<String>>, Error> {
  Builder::new()
    .status(200)
    .body(None)
    .map_err(|e| Error::new(ErrorKind::Other, e))
}

#[cfg(test)]
mod test {
  use crate::configuration::Configuration;
  use crate::persistence::RecordStore;

  #[test]
  fn existing_user_ok() {
    let config = Configuration::load("krumnet-config.example.json").unwrap();
    let records = RecordStore::open(&config).unwrap();
    assert_eq!(true, true);
  }
}
