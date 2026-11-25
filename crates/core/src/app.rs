//! Everything Famulen holds, opened once and shared by the API, the session
//! and the chores.

use std::path::PathBuf;
use std::sync::Arc;

use crate::calendars::Calendars;
use crate::connections::Connections;
use crate::console_log::ConsoleLog;
use crate::journal::Journal;
use crate::lookout::Feed;
use crate::models::Models;
use crate::permissions::Permissions;
use crate::records::Records;
use crate::schedule::Scheduler;
use crate::settings::Settings;
use crate::store::Store;
use crate::workgroups::Workgroups;

/// Answers an approval the owner gave on the console: true when it was
/// still waiting.
pub type Answerer = Arc<
    dyn Fn(
            i64,
            crate::approvals::Answer,
            Vec<String>,
        ) -> futures_util::future::BoxFuture<'static, anyhow::Result<bool>>
        + Send
        + Sync,
>;

/// Everything the console reads and changes.
#[derive(Clone)]
pub struct App {
    pub store: Store,
    pub records: Records,
    pub workgroups: Workgroups,
    pub calendars: Calendars,
    pub connections: Connections,
    pub console: ConsoleLog,
    pub attachments: crate::attachments::Attachments,
    pub permissions: Permissions,
    pub models: Models,
    pub workers: crate::workers::Workers,
    pub lookout: crate::lookout::Lookout,
    pub triage: crate::triage::Triage,
    pub notices: crate::notices::Notices,
    pub journal: Journal,
    pub settings: Settings,
    pub memory_dir: PathBuf,
    pub notebook: crate::notebook::Notebook,
    /// The notebook's index, to search it.
    pub index: famulen_notebook::index::Index,
    pub notebook_place: crate::notebook_place::NotebookPlace,
    /// Told when a setting changed, so the engine is built again with it.
    pub changed: Arc<tokio::sync::Notify>,
    /// Told when an input was queued, so the session takes it.
    pub inputs: Arc<tokio::sync::Notify>,
    pub http: reqwest::Client,
    /// The one door to the outside.
    pub gateway: crate::gateway::Gateway,
    pub answerer: Arc<std::sync::Mutex<Option<Answerer>>>,
    /// Where the owner opens the console, if set; logins come back there.
    pub console_url: Option<String>,
    /// The engine's Slack clients, while Slack is connected.
    pub slack: Arc<
        std::sync::RwLock<
            Option<Arc<crate::slack::clients::SlackClients<crate::slack::web::GatewaySlack>>>,
        >,
    >,
    /// Held while the Slack apps' own icons are set.
    pub slack_app_icons: crate::slack::app_icon::AppIcons,
    /// How the secretary's posts on Slack show it.
    pub slack_looks: crate::slack::looks::Looks,
    /// Where the gateway's receivers are reached from outside, if the owner
    /// made them reachable; calendar changes are asked to be sent there.
    pub receive_url: Option<String>,
}

impl App {
    pub fn tz(&self) -> chrono_tz::Tz {
        self.settings.timezone()
    }

    pub fn scheduler(&self) -> Scheduler {
        Scheduler::new(self.store.clone(), self.records.clone(), self.tz())
    }

    /// The secretary's name, which the owner chooses; ファム until then.
    pub fn aide_name(&self) -> String {
        crate::persona::name(&self.store)
    }

    /// Register the models this machine offers now. A source that cannot be
    /// asked, such as a program that is not installed, changes nothing.
    pub async fn find_models(&self) -> anyhow::Result<()> {
        use crate::models::{Cli, OLLAMA, Source, claude_models, codex_models, ollama_models};
        let claude = self.settings.effective()?.claude.bin;
        let (claude, codex, ollama) = tokio::join!(
            claude_models(&claude),
            codex_models(Cli::Codex.executable()),
            ollama_models(&self.http, OLLAMA),
        );
        for (source, offered) in [
            (Source::Claude, claude),
            (Source::Codex, codex),
            (Source::Ollama, ollama),
        ] {
            match offered {
                Ok(offered) => self.models.take_in(source, offered)?,
                Err(error) => {
                    tracing::debug!(source = source.as_str(), %error, "could not list models");
                }
            }
        }
        Ok(())
    }

    /// Read every Google calendar again.
    pub async fn refresh_calendars(&self) {
        let tz = self.tz();
        let Ok(calendars) = self.calendars.list() else {
            return;
        };
        for c in calendars {
            if let Some((conn, id)) = &c.google {
                self.read_google_calendar(&c, conn, id, tz).await;
            }
        }
    }

    /// Read a Google calendar straight from Google and keep its events; a
    /// failure is kept on the calendar and the events read before stay.
    async fn read_google_calendar(
        &self,
        c: &crate::calendars::Calendar,
        conn: &str,
        calendar: &str,
        tz: chrono_tz::Tz,
    ) {
        let read = async {
            let (from, to) = crate::calendars::Calendars::window(chrono::Utc::now());
            let pages = self.google_events(conn, calendar, from, to).await?;
            let events = crate::calendars::google_events(&pages, tz)?;
            self.calendars.store_events(&c.id, &events)
        };
        if let Err(error) = read.await {
            tracing::warn!(calendar = %c.name, %error, "could not read the Google calendar");
            let _ = self.calendars.store_error(&c.id, error.to_string());
        }
    }

    /// Once a connection is logged in: a Google account's primary calendar
    /// goes on the calendar, and what arrives for a Google account or a
    /// Slack workspace is watched, unless that was set up before. Its
    /// receivers start at once.
    pub fn adopt_account(&self, conn: &str) -> anyhow::Result<()> {
        let Some(c) = self.connections.get(conn)? else {
            return Ok(());
        };
        let who = crate::channel::account_name(&c);
        let feed = self.feed_of(conn)?;
        let google = feed == Feed::Mail;
        let slack = feed == Feed::Chat;
        if google {
            let colors = crate::workgroups::COLORS;
            let color = colors[self.calendars.list()?.len() % colors.len()].1;
            self.calendars.add(
                crate::notebook::Writer::Secretary,
                &who,
                Some((conn, "primary")),
                color,
            )?;
            self.refresh_calendars_later();
        }
        if google || slack {
            let watched = self
                .lookout
                .sources()?
                .iter()
                .any(|s| s.connection == conn && s.tool == crate::lookout::ARRIVED);
            if !watched {
                let name = if google {
                    format!("Gmail · {who}")
                } else {
                    format!("Slack · {who}")
                };
                self.lookout.add_source(
                    name,
                    conn,
                    crate::lookout::ARRIVED,
                    &serde_json::json!({}),
                    feed,
                    None,
                )?;
            }
        }
        crate::runtime::sync_receivers(self)
    }

    /// What `conn` brings, which decides how often a source on it is read
    /// until the owner chooses.
    pub fn feed_of(&self, conn: &str) -> anyhow::Result<Feed> {
        let Some(c) = self.connections.get(conn)? else {
            return Ok(Feed::Other);
        };
        Ok(if crate::arrivals::is_google(&c) {
            Feed::Mail
        } else if crate::channel::is_slack(&c) {
            Feed::Chat
        } else {
            Feed::Other
        })
    }

    /// Refresh the calendars without making a request wait for it.
    pub fn refresh_calendars_later(&self) {
        let app = self.clone();
        tokio::spawn(async move {
            app.refresh_calendars().await;
            app.console.notify(crate::console_log::Event::Records);
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::lookout::{ARRIVED, Feed};

    #[tokio::test]
    async fn an_adopted_account_is_watched_every_half_hour_for_mail_and_every_five_minutes_for_chat()
     {
        let dir = crate::notebook::testing::TempDir::new("app-adopt-feed");
        let app = crate::runtime::testing::app(dir.path());
        let mail = app
            .gateway
            .add_defined(crate::arrivals::GOOGLE, Default::default(), None)
            .unwrap();
        let chat = app
            .gateway
            .add_defined(
                crate::channel::DEFINITION,
                [("user".to_owned(), "U1".to_owned())].into(),
                None,
            )
            .unwrap();
        app.adopt_account(&mail.id).unwrap();
        app.adopt_account(&chat.id).unwrap();

        let every = |conn: &str| {
            app.lookout
                .sources()
                .unwrap()
                .into_iter()
                .find(|s| s.connection == conn && s.tool == ARRIVED)
                .map(|s| s.interval)
        };
        assert_eq!(every(&mail.id), Some(Feed::Mail.every()));
        assert_eq!(every(&chat.id), Some(Feed::Chat.every()));
        assert_eq!(app.feed_of("nowhere").unwrap(), Feed::Other);
    }

    #[test]
    fn the_secretary_is_called_fam_until_the_owner_names_it() {
        let dir = crate::notebook::testing::TempDir::new("app-aide-name");
        let app = crate::runtime::testing::app(dir.path());
        assert_eq!(app.aide_name(), "ファム");

        app.store.set_state("persona.name", "ミナ").unwrap();
        assert_eq!(app.aide_name(), "ミナ");
    }
}
