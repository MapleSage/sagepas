const crypto = require("crypto");

const OBJECT_TYPES = {
  contact: "contacts",
  company: "companies",
  deal: "deals",
  ticket: "tickets",
};

const STANDARD_PROPERTIES = {
  contact: ["firstname", "lastname", "email", "phone", "company"],
  company: ["name", "domain", "phone"],
  deal: ["dealname", "dealstage", "amount", "closedate"],
  ticket: ["subject", "hs_pipeline_stage", "content"],
};

const COMMON_SAGEPAS_PROPERTIES = [
  "sagepas_customer_id",
  "sagepas_policy_id",
  "sagepas_policy_number",
  "sagepas_policy_status",
  "sagepas_sync_status",
  "sagepas_last_synced_at",
];

const SAGEPAS_PROPERTIES_BY_OBJECT = {
  contact: new Set([
    ...COMMON_SAGEPAS_PROPERTIES,
    "sagepas_quote_id",
    "sagepas_quote_status",
  ]),
  company: new Set([
    ...COMMON_SAGEPAS_PROPERTIES,
    "sagepas_premium",
    "sagepas_currency",
  ]),
  deal: new Set([
    ...COMMON_SAGEPAS_PROPERTIES,
    "sagepas_quote_id",
    "sagepas_quote_status",
    "sagepas_premium",
    "sagepas_currency",
  ]),
  ticket: new Set([...COMMON_SAGEPAS_PROPERTIES, "sagepas_servicing_context"]),
};

function response(statusCode, body) {
  return {
    statusCode,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
}

function header(context, name) {
  const headers = context?.headers || context?.request?.headers || {};
  const match = Object.keys(headers).find(
    (key) => key.toLowerCase() === name.toLowerCase(),
  );
  const value = match ? headers[match] : undefined;
  return Array.isArray(value) ? value[0] : value;
}

function authorized(context) {
  const expected = process.env.SAGEPAS_SYNC_SECRET;
  const bridgeHeader = header(context, "x-sagepas-bridge-secret");
  const bearerHeader = String(header(context, "authorization") || "").replace(
    /^Bearer\s+/i,
    "",
  );
  const supplied = String(bridgeHeader || bearerHeader || "");
  if (!expected || !supplied) return false;
  const left = Buffer.from(expected);
  const right = Buffer.from(supplied);
  return left.length === right.length && crypto.timingSafeEqual(left, right);
}

function bodyOf(context) {
  const body = context?.body ?? context?.request?.body ?? {};
  if (typeof body === "string") return JSON.parse(body || "{}");
  return body || {};
}

// The caller-supplied portal_id is checked against accountId -- the real
// invoking portal, which HubSpot injects into every serverless function
// call and which this code never chooses or stores. That makes this
// function portal-agnostic by construction: the same deployed code is
// correct in the dev/test portal, the production portal, and any future
// one, with nothing to update at promotion time. (Previously this checked
// against a portal ID hardcoded in this file, which was correct only for
// the portal it happened to be written against -- work order item 9.)
function identity(body, accountId) {
  const portalId = Number(body.portal_id ?? body.portalId);
  const objectType = String(body.object_type ?? body.objectType ?? "")
    .trim()
    .toLowerCase();
  const objectId = String(body.object_id ?? body.objectId ?? "").trim();
  if (!Number.isSafeInteger(portalId) || portalId <= 0) {
    throw new Error("portalId must be a positive integer");
  }
  if (portalId !== accountId) {
    throw new Error(`portalId must match the invoking HubSpot account (${accountId})`);
  }
  if (!OBJECT_TYPES[objectType]) {
    throw new Error("objectType must be contact, company, deal, or ticket");
  }
  if (!objectId) throw new Error("objectId is required");
  return { portalId, objectType, objectId };
}

// Same portal/objectType validation as identity(), minus objectId --
// used by create(), where the object doesn't exist yet.
function identityForCreate(body, accountId) {
  const portalId = Number(body.portal_id ?? body.portalId);
  const objectType = String(body.object_type ?? body.objectType ?? "")
    .trim()
    .toLowerCase();
  if (!Number.isSafeInteger(portalId) || portalId <= 0) {
    throw new Error("portalId must be a positive integer");
  }
  if (portalId !== accountId) {
    throw new Error(`portalId must match the invoking HubSpot account (${accountId})`);
  }
  if (!OBJECT_TYPES[objectType]) {
    throw new Error("objectType must be contact, company, deal, or ticket");
  }
  return { portalId, objectType };
}

function objectValue(value) {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value
    : {};
}

function setProperty(properties, allowed, name, value) {
  if (!allowed.has(name) || value === undefined || value === null) return;
  properties[name] = String(value);
}

function projectionProperties(objectType, payload) {
  const projection = objectValue(payload);
  const link = objectValue(projection.link);
  const customer = objectValue(projection.customer);
  const quote = objectValue(projection.quote);
  const policy = objectValue(projection.policy);
  const allowed = SAGEPAS_PROPERTIES_BY_OBJECT[objectType];
  const properties = {};

  setProperty(
    properties,
    allowed,
    "sagepas_customer_id",
    customer.id ?? link.customer_id,
  );
  setProperty(
    properties,
    allowed,
    "sagepas_quote_id",
    quote.id ?? link.quote_id,
  );
  setProperty(properties, allowed, "sagepas_quote_status", quote.state);
  setProperty(
    properties,
    allowed,
    "sagepas_policy_id",
    policy.id ?? link.policy_id,
  );
  setProperty(
    properties,
    allowed,
    "sagepas_policy_number",
    policy.policy_number,
  );
  setProperty(properties, allowed, "sagepas_policy_status", policy.state);
  setProperty(
    properties,
    allowed,
    "sagepas_premium",
    policy.premium ?? quote.premium,
  );
  setProperty(
    properties,
    allowed,
    "sagepas_currency",
    policy.currency ?? quote.currency,
  );

  if (objectType === "ticket") {
    const parts = [
      policy.policy_number ? `Policy ${policy.policy_number}` : null,
      policy.state ? `status ${policy.state}` : null,
      projection.reason ? `reason ${projection.reason}` : null,
    ].filter(Boolean);
    setProperty(
      properties,
      allowed,
      "sagepas_servicing_context",
      parts.length ? parts.join(" · ") : "SagePAS policy servicing",
    );
  }

  return properties;
}

async function hubspot(path, init = {}) {
  const token = process.env.PRIVATE_APP_ACCESS_TOKEN;
  if (!token) throw new Error("HubSpot app access token is unavailable");
  const result = await fetch(`https://api.hubapi.com${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/json",
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...(init.headers || {}),
    },
  });
  const text = await result.text();
  let data = {};
  try {
    data = text ? JSON.parse(text) : {};
  } catch {
    data = { message: text };
  }
  if (!result.ok) {
    const error = new Error(
      data.message || `HubSpot API returned ${result.status}`,
    );
    error.status = result.status;
    throw error;
  }
  return data;
}

async function pull(body, accountId) {
  const { portalId, objectType, objectId } = identity(body, accountId);
  const properties = [
    ...STANDARD_PROPERTIES[objectType],
    ...SAGEPAS_PROPERTIES_BY_OBJECT[objectType],
  ];
  const query = new URLSearchParams({ properties: properties.join(",") });
  const record = await hubspot(
    `/crm/v3/objects/${OBJECT_TYPES[objectType]}/${encodeURIComponent(objectId)}?${query}`,
  );
  return {
    portalId,
    objectType,
    objectId,
    properties: record.properties || {},
    updatedAt: record.updatedAt || null,
  };
}

async function writeProperties(body, submitted, accountId) {
  const { portalId, objectType, objectId } = identity(body, accountId);
  if (!submitted || typeof submitted !== "object" || Array.isArray(submitted)) {
    throw new Error("properties must be an object");
  }
  const allowed = SAGEPAS_PROPERTIES_BY_OBJECT[objectType];
  const properties = {};
  for (const [name, value] of Object.entries(submitted)) {
    if (!allowed.has(name)) {
      throw new Error(
        `unsupported SagePAS property for ${objectType}: ${name}`,
      );
    }
    properties[name] = value == null ? "" : String(value);
  }
  properties.sagepas_last_synced_at = new Date().toISOString();
  properties.sagepas_sync_status = "synced";
  const record = await hubspot(
    `/crm/v3/objects/${OBJECT_TYPES[objectType]}/${encodeURIComponent(objectId)}`,
    { method: "PATCH", body: JSON.stringify({ properties }) },
  );
  return {
    portalId,
    objectType,
    objectId,
    updatedAt: record.updatedAt || null,
    properties,
  };
}

async function push(body, accountId) {
  return writeProperties(body, body.properties, accountId);
}

// Unlike push/upsert (which sync the fixed sagepas_* property set), create
// accepts whatever standard + custom properties the caller supplies -- it's
// a general "make a new CRM object" primitive, not scoped to SagePAS's own
// properties. Trust boundary is the shared secret, same as every other op.
async function create(body, accountId) {
  const { objectType } = identityForCreate(body, accountId);
  const properties = objectValue(body.properties);
  if (Object.keys(properties).length === 0) {
    throw new Error("properties must be a non-empty object");
  }
  const stringified = {};
  for (const [name, value] of Object.entries(properties)) {
    if (value === undefined || value === null) continue;
    stringified[name] = String(value);
  }
  const record = await hubspot(`/crm/v3/objects/${OBJECT_TYPES[objectType]}`, {
    method: "POST",
    body: JSON.stringify({ properties: stringified }),
  });
  return {
    objectType,
    objectId: record.id,
    properties: record.properties || {},
  };
}

// Canonical identity resolution: which HubSpot contact is this email?
// FNOL, UW, and PAS's own auto-link path all call this instead of doing
// their own independent search-then-create -- one contact per email,
// regardless of which system encounters that person first. Searches before
// creating; never creates a second contact for an email that already has one.
async function findOrCreateContactByEmail(body) {
  const email = String(body.email ?? "").trim().toLowerCase();
  if (!email) throw new Error("email is required");

  const searchResult = await hubspot("/crm/v3/objects/contacts/search", {
    method: "POST",
    body: JSON.stringify({
      filterGroups: [
        { filters: [{ propertyName: "email", operator: "EQ", value: email }] },
      ],
      properties: ["email", "firstname", "lastname", "phone"],
      limit: 1,
    }),
  });

  const existing = (searchResult.results || [])[0];
  if (existing) {
    return {
      objectId: existing.id,
      created: false,
      properties: existing.properties || {},
    };
  }

  const extra = objectValue(body.properties);
  const properties = { email };
  for (const [name, value] of Object.entries(extra)) {
    if (value === undefined || value === null || value === "") continue;
    properties[name] = String(value);
  }

  const record = await hubspot("/crm/v3/objects/contacts", {
    method: "POST",
    body: JSON.stringify({ properties }),
  });

  return {
    objectId: record.id,
    created: true,
    properties: record.properties || {},
  };
}

const ASSOCIATION_TYPE_ID = {
  "ticket-contact": 16,
  "ticket-company": 26,
  "deal-contact": 3,
  "deal-company": 5,
};

async function associate(body) {
  const fromType = String(body.from_type ?? body.fromType ?? "").trim().toLowerCase();
  const fromId = String(body.from_id ?? body.fromId ?? "").trim();
  const toType = String(body.to_type ?? body.toType ?? "").trim().toLowerCase();
  const toId = String(body.to_id ?? body.toId ?? "").trim();
  if (!OBJECT_TYPES[fromType] || !OBJECT_TYPES[toType]) {
    throw new Error("from_type/to_type must be contact, company, deal, or ticket");
  }
  if (!fromId || !toId) throw new Error("from_id and to_id are required");

  const typeKey = `${fromType}-${toType}`;
  const associationTypeId = ASSOCIATION_TYPE_ID[typeKey];
  if (!associationTypeId) {
    throw new Error(`unsupported association: ${typeKey}`);
  }

  await hubspot(
    `/crm/v4/objects/${OBJECT_TYPES[fromType]}/${encodeURIComponent(fromId)}/associations/${OBJECT_TYPES[toType]}/${encodeURIComponent(toId)}`,
    {
      method: "PUT",
      body: JSON.stringify([
        {
          associationCategory: "HUBSPOT_DEFINED",
          associationTypeId,
        },
      ]),
    },
  );
  return { fromType, fromId, toType, toId };
}

async function upsert(body, accountId) {
  if (!body.event_id || typeof body.event_id !== "string") {
    throw new Error("event_id is required");
  }
  const { objectType } = identity(body, accountId);
  const result = await writeProperties(
    body,
    projectionProperties(objectType, body.payload),
    accountId,
  );
  return { ...result, eventId: body.event_id };
}

exports.main = async (context = {}) => {
  if (!authorized(context)) {
    return response(401, { message: "unauthorized" });
  }

  try {
    const body = bodyOf(context);
    if (body.operation === "health") {
      return response(200, { status: "ok" });
    }
    // accountId is HubSpot's own record of which portal is invoking this
    // function right now -- resolved per-call, not configured, so it can't
    // go stale the way a hardcoded portal ID could. Only the operations
    // that write/read a specific CRM object need it (identity() checks
    // caller-supplied portal_id against it); find_or_create_contact_by_email
    // and associate operate on whatever portal owns this deployment's own
    // access token and don't take a portal_id at all.
    const accountId = Number(context.accountId);
    if (
      ["pull", "push", "upsert", "create"].includes(body.operation) &&
      (!Number.isSafeInteger(accountId) || accountId <= 0)
    ) {
      return response(500, {
        message: "unable to resolve the invoking HubSpot account",
      });
    }
    if (body.operation === "pull") {
      return response(200, await pull(body, accountId));
    }
    if (body.operation === "push") {
      return response(200, await push(body, accountId));
    }
    if (body.operation === "upsert") {
      return response(200, await upsert(body, accountId));
    }
    if (body.operation === "create") {
      return response(201, await create(body, accountId));
    }
    if (body.operation === "associate") {
      return response(200, await associate(body));
    }
    if (body.operation === "find_or_create_contact_by_email") {
      return response(200, await findOrCreateContactByEmail(body));
    }
    return response(400, {
      message:
        "operation must be health, pull, push, upsert, create, associate, or find_or_create_contact_by_email",
    });
  } catch (error) {
    const status = Number(error?.status);
    return response(
      Number.isInteger(status) && status >= 400 && status < 600 ? status : 400,
      { message: error instanceof Error ? error.message : "sync failed" },
    );
  }
};
