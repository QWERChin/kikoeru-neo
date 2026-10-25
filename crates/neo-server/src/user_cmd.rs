//! `kikoeru-neo user ...`: local account maintenance, as listed in
//! book/src/auth.md. There is no web route for creating the first admin.

use std::io::{self, BufRead};

use anyhow::{Context, Result, bail};
use neo_db::{
    self as db, Db,
    users::{self, CreateError, DeleteOutcome},
};

use crate::{
    cli::{PasswordSource, UserCommand},
    password, token,
};

const MAX_NAME_CHARS: usize = 64;

pub async fn run(db: &Db, command: UserCommand) -> Result<String> {
    match command {
        UserCommand::Create {
            name,
            admin,
            password,
        } => {
            validate_name(&name)?;
            let hash = hash(&read_password(&password)?)?;
            match users::create(
                db,
                &name,
                &hash,
                admin,
                token::initial_version()?,
                db::now_ms(),
            )
            .await
            {
                Ok(_) => Ok(format!(
                    "created {} {name}",
                    if admin { "admin" } else { "user" }
                )),
                Err(CreateError::NameTaken(_)) => bail!("user {name} already exists"),
                Err(CreateError::Db(e)) => Err(e.into()),
            }
        }
        UserCommand::SetPassword { name, password } => {
            let user = users::find_by_name(db, &name)
                .await?
                .with_context(|| format!("no user named {name}"))?;
            let hash = hash(&read_password(&password)?)?;
            if !users::set_password(db, user.id, &hash).await? {
                bail!("no user named {name}");
            }
            Ok(format!(
                "set password for {name}; existing tokens are revoked"
            ))
        }
        UserCommand::Delete { name } => match users::delete(db, &name).await? {
            DeleteOutcome::Deleted => Ok(format!("deleted user {name}")),
            DeleteOutcome::NotFound => bail!("no user named {name}"),
            DeleteOutcome::LastAdmin => {
                bail!("{name} is the last admin; create another admin before deleting it")
            }
        },
    }
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        bail!(
            "invalid user name {name:?}: use 1 to {MAX_NAME_CHARS} characters without control characters or surrounding spaces"
        );
    }
    Ok(())
}

pub(crate) fn hash(password: &str) -> Result<String> {
    if !password::is_long_enough(password) {
        bail!(
            "password must be at least {} characters",
            password::MIN_LENGTH
        );
    }
    password::hash(password).map_err(|e| anyhow::anyhow!("cannot hash password: {e}"))
}

fn read_password(source: &PasswordSource) -> Result<String> {
    if source.password_stdin {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        let password = line.strip_suffix('\n').unwrap_or(&line);
        let password = password.strip_suffix('\r').unwrap_or(password);
        return Ok(password.to_owned());
    }
    let first = rpassword::prompt_password("Password: ")?;
    let second = rpassword::prompt_password("Repeat password: ")?;
    if first != second {
        bail!("passwords do not match");
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in ["admin", "管理员", "a.b-c_d"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", " admin", "admin ", "a\nb", &"x".repeat(65)] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }
}
