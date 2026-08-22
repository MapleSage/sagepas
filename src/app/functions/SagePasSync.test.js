const test = require("node:test");
const assert = require("node:assert/strict");

const { main } = require("./SagePasSync");

const SECRET = "unit-test-bridge-secret";

function context(body, headers = { "x-sagepas-bridge-secret": SECRET }) {
  return { body, headers };
}

function parsed(result) {
  return JSON.parse(result.body);
}

function outbox(objectType, payload = {}) {
  return {
    event_id: "066e1c69-d941-4120-a81f-132a7f025412",
    portal_id: 51752298,
    object_type: objectType,
    object_id: "12345",
    operation: "upsert",
    payload,
  };
}

test.beforeEach(() => {
  process.env.SAGEPAS_SYNC_SECRET = SECRET;
  process.env.PRIVATE_APP_ACCESS_TOKEN = "test-private-app-token";
});

test.afterEach(() => {
  delete global.fetch;
});

test("accepts the Rust bridge header and rejects an invalid secret", async () => {
  const ok = await main(context({ operation: "health" }));
  assert.equal(ok.statusCode, 200);
  assert.deepEqual(parsed(ok), { status: "ok" });

  const denied = await main(
    context({ operation: "health" }, { "x-sagepas-bridge-secret": "wrong" }),
  );
  assert.equal(denied.statusCode, 401);
});

test("translates a Rust Deal outbox snapshot into Deal properties", async () => {
  let request;
  global.fetch = async (url, init) => {
    request = { url, init };
    return {
      ok: true,
      status: 200,
      text: async () => JSON.stringify({ updatedAt: "2026-07-18T00:00:00Z" }),
    };
  };

  const result = await main(
    context(
      outbox("deal", {
        reason: "policy_changed",
        link: { customer_id: "customer-link", quote_id: "quote-link" },
        customer: { id: "customer-1" },
        quote: {
          id: "quote-1",
          state: "bound",
          premium: 1800,
          currency: "USD",
        },
        policy: {
          id: "policy-1",
          policy_number: "SAGE-1001",
          state: "active",
          premium: 1800,
          currency: "USD",
        },
      }),
    ),
  );

  assert.equal(result.statusCode, 200);
  assert.equal(
    request.url,
    "https://api.hubapi.com/crm/v3/objects/deals/12345",
  );
  assert.equal(request.init.method, "PATCH");
  const properties = JSON.parse(request.init.body).properties;
  assert.equal(properties.sagepas_customer_id, "customer-1");
  assert.equal(properties.sagepas_quote_id, "quote-1");
  assert.equal(properties.sagepas_quote_status, "bound");
  assert.equal(properties.sagepas_policy_id, "policy-1");
  assert.equal(properties.sagepas_policy_number, "SAGE-1001");
  assert.equal(properties.sagepas_policy_status, "active");
  assert.equal(properties.sagepas_premium, "1800");
  assert.equal(properties.sagepas_currency, "USD");
  assert.equal(properties.sagepas_sync_status, "synced");
  assert.ok(properties.sagepas_last_synced_at);
  assert.equal(parsed(result).eventId, outbox("deal").event_id);
});

test("filters outbound properties to those created for each CRM object", async () => {
  const bodies = [];
  global.fetch = async (_url, init) => {
    bodies.push(JSON.parse(init.body).properties);
    return { ok: true, status: 200, text: async () => "{}" };
  };

  const payload = {
    reason: "record_link_changed",
    customer: { id: "customer-1" },
    quote: { id: "quote-1", state: "quoted", premium: 250, currency: "USD" },
    policy: {
      id: "policy-1",
      policy_number: "SAGE-2002",
      state: "active",
      premium: 300,
      currency: "USD",
    },
  };
  for (const objectType of ["contact", "company", "ticket"]) {
    const result = await main(context(outbox(objectType, payload)));
    assert.equal(result.statusCode, 200);
  }

  assert.equal(bodies[0].sagepas_quote_id, "quote-1");
  assert.equal(bodies[0].sagepas_premium, undefined);
  assert.equal(bodies[1].sagepas_quote_id, undefined);
  assert.equal(bodies[1].sagepas_premium, "300");
  assert.equal(bodies[2].sagepas_quote_id, undefined);
  assert.match(bodies[2].sagepas_servicing_context, /Policy SAGE-2002/);
});

test("rejects cross-portal dispatch and missing event IDs without calling HubSpot", async () => {
  let calls = 0;
  global.fetch = async () => {
    calls += 1;
    throw new Error("must not be called");
  };

  const wrongPortal = outbox("deal");
  wrongPortal.portal_id = 3475345;
  assert.equal((await main(context(wrongPortal))).statusCode, 400);

  const missingEvent = outbox("deal");
  delete missingEvent.event_id;
  assert.equal((await main(context(missingEvent))).statusCode, 400);
  assert.equal(calls, 0);
});

test("create posts a new object with the given properties and returns its id", async () => {
  let request;
  global.fetch = async (url, init) => {
    request = { url, init };
    return {
      ok: true,
      status: 201,
      text: async () =>
        JSON.stringify({ id: "999888777", properties: { subject: "FNOL - ABC123" } }),
    };
  };

  const result = await main(
    context({
      operation: "create",
      portal_id: 51752298,
      object_type: "ticket",
      properties: { subject: "FNOL - ABC123", hs_ticket_priority: "HIGH" },
    }),
  );

  assert.equal(result.statusCode, 201);
  const body = parsed(result);
  assert.equal(body.objectType, "ticket");
  assert.equal(body.objectId, "999888777");
  assert.match(request.url, /\/crm\/v3\/objects\/tickets$/);
  assert.equal(request.init.method, "POST");
  const sentProperties = JSON.parse(request.init.body).properties;
  assert.equal(sentProperties.subject, "FNOL - ABC123");
  assert.equal(sentProperties.hs_ticket_priority, "HIGH");
});

test("create rejects an empty properties object without calling HubSpot", async () => {
  let calls = 0;
  global.fetch = async () => {
    calls += 1;
    throw new Error("must not be called");
  };

  const result = await main(
    context({
      operation: "create",
      portal_id: 51752298,
      object_type: "ticket",
      properties: {},
    }),
  );
  assert.equal(result.statusCode, 400);
  assert.equal(calls, 0);
});

test("associate links two objects via the v4 associations API with the correct type id", async () => {
  let request;
  global.fetch = async (url, init) => {
    request = { url, init };
    return { ok: true, status: 200, text: async () => "" };
  };

  const result = await main(
    context({
      operation: "associate",
      from_type: "ticket",
      from_id: "111",
      to_type: "contact",
      to_id: "222",
    }),
  );

  assert.equal(result.statusCode, 200);
  assert.match(request.url, /\/crm\/v4\/objects\/tickets\/111\/associations\/contacts\/222$/);
  assert.equal(request.init.method, "PUT");
  const payload = JSON.parse(request.init.body);
  assert.equal(payload[0].associationTypeId, 16);
});

test("associate rejects an unsupported object type pairing without calling HubSpot", async () => {
  let calls = 0;
  global.fetch = async () => {
    calls += 1;
    throw new Error("must not be called");
  };

  const result = await main(
    context({
      operation: "associate",
      from_type: "contact",
      from_id: "111",
      to_type: "ticket",
      to_id: "222",
    }),
  );
  assert.equal(result.statusCode, 400);
  assert.equal(calls, 0);
});

test("find_or_create_contact_by_email returns the existing contact without creating a second one", async () => {
  const requests = [];
  global.fetch = async (url, init) => {
    requests.push({ url, init });
    return {
      ok: true,
      status: 200,
      text: async () =>
        JSON.stringify({
          results: [{ id: "555111", properties: { email: "jane@example.com", firstname: "Jane" } }],
        }),
    };
  };

  const result = await main(
    context({
      operation: "find_or_create_contact_by_email",
      email: "JANE@Example.com",
    }),
  );

  assert.equal(result.statusCode, 200);
  const body = parsed(result);
  assert.equal(body.objectId, "555111");
  assert.equal(body.created, false);
  assert.equal(requests.length, 1);
  assert.match(requests[0].url, /\/crm\/v3\/objects\/contacts\/search$/);
  const searchBody = JSON.parse(requests[0].init.body);
  assert.equal(searchBody.filterGroups[0].filters[0].value, "jane@example.com");
});

test("find_or_create_contact_by_email creates a new contact when no match exists", async () => {
  const requests = [];
  global.fetch = async (url, init) => {
    requests.push({ url, init });
    if (url.endsWith("/search")) {
      return { ok: true, status: 200, text: async () => JSON.stringify({ results: [] }) };
    }
    return {
      ok: true,
      status: 201,
      text: async () =>
        JSON.stringify({ id: "999222", properties: { email: "new@example.com", firstname: "New" } }),
    };
  };

  const result = await main(
    context({
      operation: "find_or_create_contact_by_email",
      email: "new@example.com",
      properties: { firstname: "New", lastname: "Person" },
    }),
  );

  assert.equal(result.statusCode, 200);
  const body = parsed(result);
  assert.equal(body.objectId, "999222");
  assert.equal(body.created, true);
  assert.equal(requests.length, 2);
  assert.match(requests[1].url, /\/crm\/v3\/objects\/contacts$/);
  const createBody = JSON.parse(requests[1].init.body);
  assert.equal(createBody.properties.email, "new@example.com");
  assert.equal(createBody.properties.firstname, "New");
});

test("find_or_create_contact_by_email rejects a missing email without calling HubSpot", async () => {
  let calls = 0;
  global.fetch = async () => {
    calls += 1;
    throw new Error("must not be called");
  };

  const result = await main(context({ operation: "find_or_create_contact_by_email" }));
  assert.equal(result.statusCode, 400);
  assert.equal(calls, 0);
});
