"""Feeds SDK-captured wire bodies (from the diff-spike mock API) through the
real agentcat-server ingest function process_event, against an ephemeral
Postgres testcontainer. No network, no SQS, no production."""
import json
import os

from django.test import TestCase

from apps.analytics.models.events import Event, ProcessResult, SqsMessage, process_event
from apps.core.models import Organization, Project, User

CAPTURE = os.environ["CAPTURE_FILE"]


class TestIngestCapture(TestCase):
    def setUp(self):
        org = Organization.objects.create(name="Diff Spike Org")
        User.objects.create_user(email="spike@example.com", organization=org)
        self.project = Project.objects.create(name="Diff Spike", organization=org)

    def test_every_captured_body_ingests(self):
        requests = json.load(open(CAPTURE))
        bodies = [r["body"] for r in requests if r["path"] == "/v1/public/events/"]
        self.assertGreater(len(bodies), 0)
        for body in bodies:
            body = dict(body)
            for k in ("project_id", "projectId"):
                if k in body:
                    body[k] = self.project.id
            msg = SqsMessage(body=json.dumps(body), id=f"m_{body['id']}", receipt_handle="h")
            self.assertEqual(process_event(msg), ProcessResult.SUCCESS, body.get("resource_name"))
        stored = Event.objects.filter(project_id=self.project.id)
        self.assertEqual(stored.count(), len(bodies))
        by_id = {e.id: e for e in stored}
        for body in bodies:
            e = by_id[body["id"]]
            self.assertEqual(e.resource_name, body.get("resource_name"))
            self.assertEqual(bool(e.is_error), bool(body.get("is_error", False)))
            if body.get("input_tokens") is not None:
                self.assertEqual(e.input_tokens, body["input_tokens"])
        print(f"INGESTED {len(bodies)} bodies OK; event types: {sorted({e.event_type for e in stored})}")
