//! The secretary's persona, the home screen, where the secretary may write
//! and where the notebook is.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::settings::Key;
use famulen_core::writable;
use serde::{Deserialize, Serialize};

use crate::pb::*;
use crate::{Api, OrFail, changed, fail, given};

const TILES_KEY: &str = "home.tiles";
const SETUP_HIDDEN: &str = "home.setup.hidden";
const TILE_KINDS: [(&str, &str); 6] = [
    ("today", "今日"),
    ("approvals", "確認待ち"),
    ("aide", "秘書が進めている"),
    ("task", "今日と今週のタスク"),
    ("memos", "最近のメモ"),
    ("due", "期限の近い物と契約"),
];

#[derive(Serialize, Deserialize)]
struct TileDef {
    kind: String,
    wide: bool,
    tall: bool,
}

fn default_tiles() -> Vec<TileDef> {
    [
        ("today", true, true),
        ("approvals", false, false),
        ("aide", false, false),
        ("task", true, true),
        ("memos", true, false),
    ]
    .into_iter()
    .map(|(kind, wide, tall)| TileDef {
        kind: kind.into(),
        wide,
        tall,
    })
    .collect()
}

impl Api {
    fn home(&self) -> anyhow::Result<Home> {
        let tiles: Vec<TileDef> = self
            .app
            .store
            .state(TILES_KEY)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(default_tiles);
        let setup = if self.app.store.state(SETUP_HIDDEN)?.is_some() {
            Vec::new()
        } else {
            let connections = self.app.connections.list()?;
            let steps = [
                (
                    self.app
                        .store
                        .state("persona.name")?
                        .is_some_and(|n| !n.trim().is_empty()),
                    "秘書に名前を付ける",
                    "/settings",
                ),
                (
                    connections.iter().any(|c| {
                        famulen_core::arrivals::is_google(c) && c.status == "つながっている"
                    }),
                    "Google アカウントをつなぐ（カレンダーとメール）",
                    "/settings/services",
                ),
                (
                    !self.app.workspaces()?.is_empty(),
                    "Slack のワークスペースを足す",
                    "/settings/services",
                ),
            ];
            if steps.iter().all(|(done, _, _)| *done) {
                Vec::new()
            } else {
                steps
                    .into_iter()
                    .map(|(done, label, path)| SetupStep {
                        label: label.into(),
                        done,
                        path: path.into(),
                        ..Default::default()
                    })
                    .collect()
            }
        };
        Ok(Home {
            tiles: tiles
                .into_iter()
                .map(|t| Tile {
                    kind: t.kind,
                    wide: t.wide,
                    tall: t.tall,
                    ..Default::default()
                })
                .collect(),
            kinds: TILE_KINDS
                .iter()
                .map(|(k, l)| TileKind {
                    kind: (*k).into(),
                    label: (*l).into(),
                    ..Default::default()
                })
                .collect(),
            setup,
            ..Default::default()
        })
    }

    fn persona(&self) -> anyhow::Result<Persona> {
        Ok(Persona {
            name: self.app.aide_name(),
            tone: self.app.store.state("persona.tone")?.unwrap_or_default(),
            own_tone: self
                .app
                .store
                .state("persona.own_tone")?
                .unwrap_or_default(),
            timezone: self.app.tz().name().to_owned(),
            ..Default::default()
        })
    }
}

impl SettingsService for Api {
    async fn get_persona(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Persona> {
        Response::ok(self.persona().or_fail()?)
    }

    async fn update_persona(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePersonaRequest>,
    ) -> ServiceResult<Persona> {
        let r = request.to_owned_message();
        if let Some(name) = &r.name {
            if name.trim().is_empty() {
                return Err(fail("the secretary needs a name"));
            }
            self.app
                .store
                .set_state("persona.name", name.trim())
                .or_fail()?;
        }
        if let Some(tone) = &r.tone {
            self.app
                .store
                .set_state("persona.tone", tone.trim())
                .or_fail()?;
        }
        if let Some(own) = &r.own_tone {
            self.app
                .store
                .set_state("persona.own_tone", own.trim())
                .or_fail()?;
        }
        if let Some(tz) = &r.timezone {
            self.app
                .settings
                .apply(&[(Key::Timezone, given(tz))])
                .or_fail()?;
        }
        self.app.journal.record("setting", "persona changed");
        self.app.changed.notify_one();
        changed(self);
        Response::ok(self.persona().or_fail()?)
    }

    async fn get_home(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Home> {
        Response::ok(self.home().or_fail()?)
    }

    async fn set_tiles(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetTilesRequest>,
    ) -> ServiceResult<Home> {
        let tiles: Vec<TileDef> = request
            .to_owned_message()
            .tiles
            .into_iter()
            .filter(|t| TILE_KINDS.iter().any(|(k, _)| *k == t.kind))
            .map(|t| TileDef {
                kind: t.kind,
                wide: t.wide,
                tall: t.tall,
            })
            .collect();
        self.app
            .store
            .set_state(TILES_KEY, serde_json::to_string(&tiles).map_err(fail)?)
            .or_fail()?;
        Response::ok(self.home().or_fail()?)
    }

    async fn hide_setup(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.app.store.set_state(SETUP_HIDDEN, "1").or_fail()?;
        Response::ok(Empty::default())
    }

    async fn get_writable_places(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WritablePlaces> {
        Response::ok(places(writable::places(&self.app.store).or_fail()?))
    }

    async fn add_writable_place(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, WritablePlaceRequest>,
    ) -> ServiceResult<WritablePlaces> {
        let added = writable::add(&self.app.store, request.path, home().as_deref()).or_fail()?;
        self.app.journal.record(
            "setting",
            format!("secretary may write in {}", request.path),
        );
        self.app.changed.notify_one();
        Response::ok(places(added))
    }

    async fn remove_writable_place(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, WritablePlaceRequest>,
    ) -> ServiceResult<WritablePlaces> {
        let left = writable::remove(&self.app.store, request.path).or_fail()?;
        self.app.journal.record(
            "setting",
            format!("secretary may no longer write in {}", request.path),
        );
        self.app.changed.notify_one();
        Response::ok(places(left))
    }

    async fn get_notebook_place(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NotebookPlace> {
        Response::ok(notebook_place(&self.app).or_fail()?)
    }

    async fn move_notebook(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, MoveNotebookRequest>,
    ) -> ServiceResult<NotebookPlace> {
        let (place, path) = (self.app.notebook_place.clone(), request.path.to_owned());
        tokio::task::spawn_blocking(move || place.request(&path, home().as_deref()))
            .await
            .map_err(fail)?
            .or_fail()?;
        let answer = notebook_place(&self.app).or_fail()?;
        self.app.journal.record(
            "setting",
            format!("the notebook moves to {}", answer.moving_to),
        );
        self.app.changed.notify_one();
        Response::ok(answer)
    }

    async fn get_folder(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetFolderRequest>,
    ) -> ServiceResult<Folder> {
        let path = request.path.to_owned();
        let folder =
            tokio::task::spawn_blocking(move || writable::folder(&path, home().as_deref()))
                .await
                .map_err(fail)?
                .or_fail()?;
        Response::ok(Folder {
            path: folder.path.display().to_string(),
            parent: folder
                .parent
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            children: folder.children,
            ..Default::default()
        })
    }

    async fn find_folders(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, FindFoldersRequest>,
    ) -> ServiceResult<FindFoldersResponse> {
        const BUDGET: usize = 20_000;
        let name = request.name.trim().to_owned();
        if name.is_empty() || name.contains('/') {
            return Err(fail("give a folder's name alone"));
        }
        let Some(root) = home() else {
            return Err(fail("no home to look in; write the whole path"));
        };
        let found = tokio::task::spawn_blocking(move || writable::find(&root, &name, BUDGET))
            .await
            .map_err(fail)?;
        Response::ok(FindFoldersResponse {
            paths: found.iter().map(|p| p.display().to_string()).collect(),
            ..Default::default()
        })
    }
}

fn notebook_place(app: &famulen_core::app::App) -> anyhow::Result<NotebookPlace> {
    let status = app.notebook_place.status()?;
    let shown = |path: Option<std::path::PathBuf>| {
        path.map(|p| p.display().to_string()).unwrap_or_default()
    };
    Ok(NotebookPlace {
        path: shown(Some(status.current)),
        default_path: shown(Some(status.default)),
        moving_to: shown(status.moving),
        failed: status.failed.unwrap_or_default(),
        leftover: shown(status.leftover),
        ..Default::default()
    })
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

fn places(paths: Vec<std::path::PathBuf>) -> WritablePlaces {
    WritablePlaces {
        paths: paths.iter().map(|p| p.display().to_string()).collect(),
        ..Default::default()
    }
}
