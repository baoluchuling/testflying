from __future__ import annotations

from queue import Queue
from threading import Thread

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.active_connector import active_connector_hub
from testflying_api.schema import StoreConnector
from testflying_api.seed import seed_demo_catalog
from testflying_api.store_sync import StoreConnectorClient


def test_active_connector_client_dispatches_health_to_polling_agent(
    db_session: Session,
) -> None:
    active_connector_hub.reset()
    seed_demo_catalog(db_session)
    connector = db_session.query(StoreConnector).one()
    connector.base_url = "active://account-apple-enterprise"
    connector.auth_token = "active-token"
    db_session.commit()

    results: Queue[dict[str, object] | BaseException] = Queue()

    def call_health() -> None:
        try:
            results.put(StoreConnectorClient().health(connector))
        except BaseException as error:  # pragma: no cover - surfaced by assertion below
            results.put(error)

    worker = Thread(target=call_health)
    worker.start()

    task = active_connector_hub.poll(account_id=connector.developer_account_id, timeout_seconds=2)
    assert task is not None
    assert task.method == "GET"
    assert task.path == "/health"
    assert "Authorization" not in task.headers

    completed = active_connector_hub.complete(
        task_id=task.id,
        status_code=200,
        body='{"status":"ok","developerAccountId":"account-apple-enterprise"}',
    )

    worker.join(timeout=2)
    result = results.get_nowait()
    assert completed is True
    assert result == {"status": "ok", "developerAccountId": "account-apple-enterprise"}
    active_connector_hub.reset()


def test_connector_agent_poll_requires_active_connector_and_updates_status(
    client: TestClient,
    db_session: Session,
) -> None:
    active_connector_hub.reset()
    seed_demo_catalog(db_session)
    connector = db_session.query(StoreConnector).one()
    connector.base_url = "active://account-apple-enterprise"
    connector.auth_token = "active-token"
    connector.status = "unknown"
    connector.last_checked_at = None
    db_session.commit()

    response = client.post(
        "/connector-agent/v1/poll",
        headers={"Authorization": "Bearer active-token"},
        json={"accountId": "account-apple-enterprise", "timeoutSeconds": 0},
    )

    db_session.refresh(connector)
    assert response.status_code == 200
    assert response.json() == {"task": None}
    assert connector.status == "ok"
    assert connector.last_checked_at is not None
    active_connector_hub.reset()


def test_connector_agent_rejects_wrong_token(
    client: TestClient,
    db_session: Session,
) -> None:
    active_connector_hub.reset()
    seed_demo_catalog(db_session)
    connector = db_session.query(StoreConnector).one()
    connector.base_url = "active://account-apple-enterprise"
    connector.auth_token = "active-token"
    db_session.commit()

    response = client.post(
        "/connector-agent/v1/poll",
        headers={"Authorization": "Bearer wrong"},
        json={"accountId": "account-apple-enterprise", "timeoutSeconds": 0},
    )

    assert response.status_code == 401
    active_connector_hub.reset()
