"""Feeds the publish bodies of one harness run (a driver raw file) through the
real agentcat-server ingest function process_event, against the ephemeral
Postgres testcontainer of the server's own pytest setup. Local only (R14): no
SQS, no AgentCat service, never in CI. See harness/ingest/README.md.

CAPTURE_FILE: absolute path of harness/out/<scenario>.<label>.<transport>.raw.json.
Every body's project_id (and its camelCase alias projectId) is replaced with
the id of a project this test creates; nothing else in the body changes.

An egress guard refuses every Python socket connection that is not a Unix
socket (the Docker daemon) or a loopback address (the testcontainer). It is
installed when pytest imports this file, before any fixture starts a container.
"""
import ipaddress
import json
import os
import socket

from django.test import TestCase

from apps.analytics.models.events import Event, ProcessResult, SqsMessage, process_event
from apps.core.models import Organization, Project, User

CAPTURE = os.environ["CAPTURE_FILE"]
PUBLISH_PATH = "/v1/public/events/"


def _is_local(sock, address):
    if sock.family == getattr(socket, "AF_UNIX", None):
        return True
    host = address[0] if isinstance(address, tuple) else address
    if not isinstance(host, str):
        return False
    if host == "localhost":
        return True
    try:
        ip = ipaddress.ip_address(host.split("%", 1)[0])
    except ValueError:
        return False
    mapped = getattr(ip, "ipv4_mapped", None)
    return ip.is_loopback or bool(mapped and mapped.is_loopback)


def _guard(real):
    def connect(self, address):
        if not _is_local(self, address):
            raise ConnectionRefusedError(f"ingest replay egress guard: blocked {address!r}")
        return real(self, address)

    return connect


socket.socket.connect = _guard(socket.socket.connect)
socket.socket.connect_ex = _guard(socket.socket.connect_ex)


class TestIngestCapture(TestCase):
    def setUp(self):
        org = Organization.objects.create(name="Harness Ingest Org")
        User.objects.create_user(email="harness-ingest@example.com", organization=org)
        self.project = Project.objects.create(name="Harness Ingest", organization=org)

    def test_every_captured_body_ingests(self):
        with open(CAPTURE, encoding="utf-8") as f:
            requests = json.load(f)
        bodies = [r["body"] for r in requests if r["method"] == "POST" and r["path"] == PUBLISH_PATH]
        self.assertGreater(len(bodies), 0, f"no publish bodies in {CAPTURE}")
        for body in bodies:
            body = dict(body)
            for k in ("project_id", "projectId"):
                if k in body:
                    body[k] = self.project.id
            msg = SqsMessage(body=json.dumps(body), id=f"m_{body['id']}", receipt_handle="h")
            label = f"{body.get('id')} {body.get('event_type')} {body.get('resource_name')}"
            self.assertEqual(process_event(msg), ProcessResult.SUCCESS, label)
        stored = Event.objects.filter(project_id=self.project.id)
        self.assertEqual(stored.count(), len(bodies))
        by_id = {e.id: e for e in stored}
        for body in bodies:
            e = by_id[body["id"]]
            self.assertEqual(e.resource_name, body.get("resource_name"))
            self.assertEqual(bool(e.is_error), bool(body.get("is_error", False)))
            if body.get("input_tokens") is not None:
                self.assertEqual(e.input_tokens, body["input_tokens"])
            if body.get("output_tokens") is not None:
                self.assertEqual(e.output_tokens, body["output_tokens"])
        print(f"INGESTED {len(bodies)} bodies OK; event types: {sorted({e.event_type for e in stored})}")
