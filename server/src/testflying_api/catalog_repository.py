from __future__ import annotations

from collections.abc import Iterable

from sqlalchemy import Select, select
from sqlalchemy.orm import Session, joinedload, selectinload

from testflying_api.schema import (
    App,
    Build,
    DeveloperAccount,
    DeveloperAccountApp,
    Device,
    DeviceBuildVisibility,
    Notification,
)


class CatalogRepository:
    def __init__(self, session: Session) -> None:
        self._session = session

    def current_device(self, device_id: str) -> Device | None:
        return self._session.get(Device, device_id)

    def devices(self) -> list[Device]:
        return list(self._session.scalars(select(Device).order_by(Device.registered_at.desc())))

    def visible_builds(self, *, device_id: str, platform: str) -> list[Build]:
        visible_build_ids = list(
            self._session.scalars(
                select(DeviceBuildVisibility.build_id).where(
                    DeviceBuildVisibility.device_id == device_id,
                )
            )
        )
        if not visible_build_ids:
            return []

        statement = (
            select(Build)
            .options(joinedload(Build.app), selectinload(Build.artifacts))
            .where(Build.id.in_(visible_build_ids), Build.platform == platform)
            .order_by(Build.uploaded_at.desc())
        )
        return list(self._session.scalars(statement))

    def apps_for_builds(self, builds: Iterable[Build]) -> list[App]:
        app_ids = {build.app_id for build in builds}
        if not app_ids:
            return []
        return list(
            self._session.scalars(
                select(App).where(App.id.in_(app_ids)).order_by(App.added_at.desc())
            )
        )

    def developer_accounts_for_apps(self, app_ids: Iterable[str]) -> list[DeveloperAccount]:
        app_id_set = set(app_ids)
        if not app_id_set:
            return []

        direct_accounts = list(
            self._session.scalars(
                select(DeveloperAccount)
                .join(App, App.developer_account_id == DeveloperAccount.id)
                .where(App.id.in_(app_id_set))
            )
        )
        legacy_accounts = list(
            self._session.scalars(
                select(DeveloperAccount)
                .join(
                    DeveloperAccountApp,
                    DeveloperAccountApp.developer_account_id == DeveloperAccount.id,
                )
                .where(DeveloperAccountApp.app_id.in_(app_id_set))
            )
        )
        accounts_by_id = {account.id: account for account in [*direct_accounts, *legacy_accounts]}
        return sorted(accounts_by_id.values(), key=lambda account: account.expires_at)

    def app_ids_for_developer_account(self, account_id: str) -> list[str]:
        direct_ids = list(
            self._session.scalars(select(App.id).where(App.developer_account_id == account_id))
        )
        legacy_ids = list(
            self._session.scalars(
                select(DeveloperAccountApp.app_id).where(
                    DeveloperAccountApp.developer_account_id == account_id,
                )
            )
        )
        merged_ids = list(dict.fromkeys([*direct_ids, *legacy_ids]))
        return merged_ids

    def notifications(self, *, types: set[str] | None = None) -> list[Notification]:
        statement: Select[tuple[Notification]] = select(Notification).order_by(
            Notification.created_at.desc()
        )
        if types:
            statement = statement.where(Notification.type.in_(types))
        return list(self._session.scalars(statement))

    def app_name(self, app_id: str) -> str:
        app = self._session.get(App, app_id)
        return app.name if app else ""
