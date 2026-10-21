"""Feeds the publish bodies of one harness run (a driver raw file) through the
real agentcat-server ingest function process_event, against the ephemeral
Postgres testcontainer of the server's own pytest setup. Local only (R14): no
SQS, no AgentCat service, never in CI. See harness/ingest/README.md.

CAPTURE_FILE: absolute path of harness/out/<scenario>.<label>.<transport>.raw.json.
Every body's project_id (and its camelCase alias projectId) is replaced with
the id of a project this test creates; nothing else in the body changes.

An egress guard is installed when pytest imports this file, before any fixture
starts a container. In Python code it refuses: a socket connect, connect_ex or
sendto to an address that is not a Unix socket (the Docker daemon) or loopback
(the testcontainer), and a getaddrinfo, gethostbyname or gethostbyname_ex
lookup of a name other than localhost or a loopback address. It does not cover
C extensions (libpq opens the Postgres connection itself), reverse lookups
(gethostbyaddr, getnameinfo), sendmsg, or the Docker daemon.
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


def _is_local_host(host):
    """True for localhost and loopback addresses (IPv4, IPv6, IPv4-mapped IPv6)."""
    if host is None:
        return True  # getaddrinfo(None, ...): the local wildcard, no lookup
    if isinstance(host, bytes):
        host = host.decode("ascii", "replace")
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


def _is_local(sock, address):
    if sock.family == getattr(socket, "AF_UNIX", None):
        return True
    return _is_local_host(address[0] if isinstance(address, tuple) else address)


def _blocked(what):
    return ConnectionRefusedError(f"ingest replay egress guard: blocked {what}")


def _guard_connect(real):
    def connect(self, address):
        if not _is_local(self, address):
            raise _blocked(repr(address))
        return real(self, address)

    return connect


def _guard_sendto(real):
    def sendto(self, *args):  # sendto(data, address) or sendto(data, flags, address)
        if not _is_local(self, args[-1]):
            raise _blocked(repr(args[-1]))
        return real(self, *args)

    return sendto


def _guard_lookup(real):
    def lookup(host, *args, **kwargs):
        if not _is_local_host(host):
            raise socket.gaierror(socket.EAI_NONAME, f"ingest replay egress guard: blocked lookup of {host!r}")
        return real(host, *args, **kwargs)

    return lookup


socket.socket.connect = _guard_connect(socket.socket.connect)
socket.socket.connect_ex = _guard_connect(socket.socket.connect_ex)
socket.socket.sendto = _guard_sendto(socket.socket.sendto)
socket.getaddrinfo = _guard_lookup(socket.getaddrinfo)
socket.gethostbyname = _guard_lookup(socket.gethostbyname)
socket.gethostbyname_ex = _guard_lookup(socket.gethostbyname_ex)


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
