//! The vault, for the console and the secretary: what is in it and putting
//! secrets in and out. No answer here ever holds a secret's value.

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::gateway::{
    DELETE_SECRET, DESCRIBE_SECRET, Holder, KEY_VAR, PUT_SECRET, VaultEntry,
};

use crate::pb::*;
use crate::permissions::{permission, permission_from};
use crate::{Api, OWNER, OrFail, SECRETARY, actor, fail, unless_held};

/// The lookout and workers read only what the secretary hands them, and the
/// vault is never handed.
fn refuse_others(ctx: &RequestContext) -> Result<(), ConnectError> {
    match actor(ctx) {
        OWNER | SECRETARY => Ok(()),
        who => Err(fail(format!("{who} may not touch the vault"))),
    }
}

impl Api {
    fn secret(&self, e: VaultEntry) -> anyhow::Result<Secret> {
        let holder = e.holder();
        let (connection, model) = match &holder {
            Some(Holder::Connection(id)) => {
                let name = self.app.connections.get(id)?.map(|c| c.name);
                (name.unwrap_or_else(|| id.clone()), String::new())
            }
            Some(Holder::Model(id)) => {
                let name = self.app.models.model(id)?.map(|m| m.spec.name);
                (String::new(), name.unwrap_or_else(|| id.clone()))
            }
            None => Default::default(),
        };
        Ok(Secret {
            name: e.name,
            by: e.by,
            created_at: e.created_at,
            updated_at: e.updated_at,
            connection,
            description: e.description,
            model,
            automatic: holder.is_some(),
            ..Default::default()
        })
    }

    /// One secret as the list shows it, after a change to it.
    fn kept_secret(&self, name: &str) -> Result<Secret, ConnectError> {
        let entry = self
            .app
            .gateway
            .vault()
            .and_then(|v| v.get(name.trim()).transpose())
            .transpose()
            .or_fail()?
            .ok_or_else(|| fail("the secret was not kept"))?;
        self.secret(entry).or_fail()
    }
}

impl VaultService for Api {
    async fn list_secrets(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListSecretsResponse> {
        refuse_others(&ctx)?;
        let gateway = &self.app.gateway;
        if gateway.vault().is_none() {
            return Response::ok(ListSecretsResponse {
                unavailable: format!("金庫の鍵（{}）が設定されていない", KEY_VAR),
                ..Default::default()
            });
        }
        let secrets = gateway
            .secrets()
            .or_fail()?
            .into_iter()
            .map(|e| self.secret(e))
            .collect::<anyhow::Result<_>>()
            .or_fail()?;
        Response::ok(ListSecretsResponse {
            secrets,
            ..Default::default()
        })
    }

    async fn set_secret_value(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetSecretValueRequest>,
    ) -> ServiceResult<Secret> {
        refuse_others(&ctx)?;
        let r = request.to_owned_message();
        let gateway = &self.app.gateway;
        unless_held(
            gateway
                .put_secret(actor(&ctx), &r.name, &r.value, &r.purpose)
                .or_fail()?,
        )?;
        self.app.changed.notify_one();
        Response::ok(self.kept_secret(&r.name)?)
    }

    async fn describe_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DescribeSecretRequest>,
    ) -> ServiceResult<Secret> {
        refuse_others(&ctx)?;
        let r = request.to_owned_message();
        unless_held(
            self.app
                .gateway
                .describe_secret(actor(&ctx), &r.name, &r.description)
                .or_fail()?,
        )?;
        self.app.changed.notify_one();
        Response::ok(self.kept_secret(&r.name)?)
    }

    async fn delete_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        refuse_others(&ctx)?;
        unless_held(
            self.app
                .gateway
                .delete_secret(actor(&ctx), request.id)
                .or_fail()?,
        )?;
        self.app.changed.notify_one();
        Response::ok(Empty::default())
    }

    async fn get_vault_permissions(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<VaultPermissions> {
        Response::ok(self.vault_permissions().or_fail()?)
    }

    async fn set_vault_permission(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetVaultPermissionRequest>,
    ) -> ServiceResult<VaultPermissions> {
        let r = request.to_owned_message();
        let tool = match r.change.as_str() {
            "put" => PUT_SECRET,
            "delete" => DELETE_SECRET,
            "describe" => DESCRIBE_SECRET,
            other => return Err(fail(format!("the vault has no change {other}"))),
        };
        let chosen = r.permission.as_known().and_then(permission_from);
        self.app
            .gateway
            .set_vault_permission(actor(&ctx), tool, chosen)
            .or_fail()?;
        Response::ok(self.vault_permissions().or_fail()?)
    }
}

impl Api {
    fn vault_permissions(&self) -> anyhow::Result<VaultPermissions> {
        let gateway = &self.app.gateway;
        let of =
            |tool| -> anyhow::Result<_> { Ok(permission(gateway.vault_permission(tool)?).into()) };
        Ok(VaultPermissions {
            put: of(PUT_SECRET)?,
            delete: of(DELETE_SECRET)?,
            describe: of(DESCRIBE_SECRET)?,
            ..Default::default()
        })
    }
}
