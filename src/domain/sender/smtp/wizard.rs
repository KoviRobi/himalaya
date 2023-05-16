use anyhow::Result;
use dialoguer::{Confirm, Input, Select};
use pimalaya_email::{
    OAuth2Config, OAuth2Method, OAuth2Scopes, PasswdConfig, SenderConfig, SmtpAuthConfig,
    SmtpConfig,
};
use pimalaya_oauth2::AuthorizationCodeGrant;
use pimalaya_secret::Secret;

use crate::{
    config::wizard::{prompt_passwd, THEME},
    wizard_log, wizard_prompt,
};

const SSL_TLS: &str = "SSL/TLS";
const STARTTLS: &str = "STARTTLS";
const NONE: &str = "None";
const PROTOCOLS: &[&str] = &[SSL_TLS, STARTTLS, NONE];

const PASSWD: &str = "Password";
const OAUTH2: &str = "OAuth 2.0";
const AUTH_MECHANISMS: &[&str] = &[PASSWD, OAUTH2];

const XOAUTH2: &str = "XOAUTH2";
const OAUTHBEARER: &str = "OAUTHBEARER";
const OAUTH2_MECHANISMS: &[&str] = &[XOAUTH2, OAUTHBEARER];

const SECRETS: &[&str] = &[KEYRING, RAW, CMD];
const KEYRING: &str = "Ask the password, then save it in my system's global keyring";
const RAW: &str = "Ask the password, then save it in the configuration file (not safe)";
const CMD: &str = "Use a shell command that exposes the password";

pub(crate) fn configure(account_name: &str, email: &str) -> Result<SenderConfig> {
    let mut config = SmtpConfig::default();

    config.host = Input::with_theme(&*THEME)
        .with_prompt("SMTP host")
        .default(format!("smtp.{}", email.rsplit_once('@').unwrap().1))
        .interact()?;

    let protocol = Select::with_theme(&*THEME)
        .with_prompt("SMTP security protocol")
        .items(PROTOCOLS)
        .default(0)
        .interact_opt()?;

    let default_port = match protocol {
        Some(idx) if PROTOCOLS[idx] == SSL_TLS => {
            config.ssl = Some(true);
            465
        }
        Some(idx) if PROTOCOLS[idx] == STARTTLS => {
            config.starttls = Some(true);
            587
        }
        _ => 25,
    };

    config.port = Input::with_theme(&*THEME)
        .with_prompt("SMTP port")
        .validate_with(|input: &String| input.parse::<u16>().map(|_| ()))
        .default(default_port.to_string())
        .interact()
        .map(|input| input.parse::<u16>().unwrap())?;

    config.login = Input::with_theme(&*THEME)
        .with_prompt("SMTP login")
        .default(email.to_owned())
        .interact()?;

    let auth = Select::with_theme(&*THEME)
        .with_prompt("SMTP authentication mechanism")
        .items(AUTH_MECHANISMS)
        .default(0)
        .interact_opt()?;

    config.auth = match auth {
        Some(idx) if AUTH_MECHANISMS[idx] == PASSWD => {
            let secret = Select::with_theme(&*THEME)
                .with_prompt("SMTP authentication strategy")
                .items(SECRETS)
                .default(0)
                .interact_opt()?;

            let config = match secret {
                Some(idx) if SECRETS[idx] == KEYRING => {
                    Secret::new_keyring(format!("{account_name}-smtp-passwd"))
                        .set(prompt_passwd("SMTP password")?)?;
                    PasswdConfig::default()
                }
                Some(idx) if SECRETS[idx] == RAW => PasswdConfig {
                    passwd: Secret::Raw(prompt_passwd("SMTP password")?),
                },
                Some(idx) if SECRETS[idx] == CMD => PasswdConfig {
                    passwd: Secret::new_cmd(
                        Input::with_theme(&*THEME)
                            .with_prompt("Shell command")
                            .default(format!("pass show {account_name}-smtp-passwd"))
                            .interact()?,
                    ),
                },
                _ => PasswdConfig::default(),
            };
            SmtpAuthConfig::Passwd(config)
        }
        Some(idx) if AUTH_MECHANISMS[idx] == OAUTH2 => {
            let mut config = OAuth2Config::default();

            let method = Select::with_theme(&*THEME)
                .with_prompt("SMTP OAuth 2.0 mechanism")
                .items(OAUTH2_MECHANISMS)
                .default(0)
                .interact_opt()?;

            config.method = match method {
                Some(idx) if OAUTH2_MECHANISMS[idx] == XOAUTH2 => OAuth2Method::XOAuth2,
                Some(idx) if OAUTH2_MECHANISMS[idx] == OAUTHBEARER => OAuth2Method::OAuthBearer,
                _ => OAuth2Method::XOAuth2,
            };

            config.client_id = Input::with_theme(&*THEME)
                .with_prompt("SMTP OAuth 2.0 client id")
                .interact()?;

            let client_secret: String = Input::with_theme(&*THEME)
                .with_prompt("SMTP OAuth 2.0 client secret")
                .interact()?;
            Secret::new_keyring(format!("{account_name}-smtp-oauth2-client-secret"))
                .set(&client_secret)?;

            config.auth_url = Input::with_theme(&*THEME)
                .with_prompt("SMTP OAuth 2.0 authorization URL")
                .interact()?;

            config.token_url = Input::with_theme(&*THEME)
                .with_prompt("SMTP OAuth 2.0 token URL")
                .interact()?;

            config.scopes = OAuth2Scopes::Scope(
                Input::with_theme(&*THEME)
                    .with_prompt("SMTP OAuth 2.0 main scope")
                    .interact()?,
            );

            while Confirm::new()
                .with_prompt(wizard_prompt!(
                    "Would you like to add more SMTP OAuth 2.0 scopes?"
                ))
                .default(false)
                .interact_opt()?
                .unwrap_or_default()
            {
                let mut scopes = match config.scopes {
                    OAuth2Scopes::Scope(scope) => vec![scope],
                    OAuth2Scopes::Scopes(scopes) => scopes,
                };

                scopes.push(
                    Input::with_theme(&*THEME)
                        .with_prompt("Additional SMTP OAuth 2.0 scope")
                        .interact()?,
                );

                config.scopes = OAuth2Scopes::Scopes(scopes);
            }

            config.pkce = Confirm::new()
                .with_prompt(wizard_prompt!(
                    "Would you like to enable PKCE verification?"
                ))
                .default(true)
                .interact_opt()?
                .unwrap_or(true);

            wizard_log!("To complete your OAuth 2.0 setup, click on the following link:");

            let mut builder = AuthorizationCodeGrant::new(
                config.client_id.clone(),
                client_secret,
                config.auth_url.clone(),
                config.token_url.clone(),
            )?;

            if config.pkce {
                builder = builder.with_pkce();
            }

            for scope in config.scopes.clone() {
                builder = builder.with_scope(scope);
            }

            let client = builder.get_client()?;
            let (redirect_url, csrf_token) = builder.get_redirect_url(&client);

            println!("{}", redirect_url.to_string());
            println!("");

            let (access_token, refresh_token) = builder.wait_for_redirection(client, csrf_token)?;

            Secret::new_keyring(format!("{account_name}-smtp-oauth2-access-token"))
                .set(access_token)?;

            if let Some(refresh_token) = &refresh_token {
                Secret::new_keyring(format!("{account_name}-smtp-oauth2-refresh-token"))
                    .set(refresh_token)?;
            }

            SmtpAuthConfig::OAuth2(config)
        }
        _ => SmtpAuthConfig::default(),
    };

    Ok(SenderConfig::Smtp(config))
}
