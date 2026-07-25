const crypto = require("crypto");

const PORTAL_ID = 51752298;
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

function identity(body) {
  const portalId = Number(body.portal_id ?? body.portalId);
  const objectType = String(body.object_type ?? body.objectType ?? "")
    .trim()
    .toLowerCase();
  const objectId = String(body.object_id ?? body.objectId ?? "").trim();
  if (!Number.isSafeInteger(portalId) || portalId <= 0) {
    throw new Error("portalId must be a positive integer");
  }
  if (portalId !== PORTAL_ID) {
    throw new Error(`portalId must be ${PORTAL_ID}`);
  }
  if (!OBJECT_TYPES[objectType]) {
    throw new Error("objectType must be contact, company, deal, or ticket");
  }
  if (!objectId) throw new Error("objectId is required");
  return { portalId, objectType, objectId };
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

async function pull(body) {
  const { portalId, objectType, objectId } = identity(body);
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

async function writeProperties(body, submitted) {
  const { portalId, objectType, objectId } = identity(body);
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

async function push(body) {
  return writeProperties(body, body.properties);
}

async function upsert(body) {
  if (!body.event_id || typeof body.event_id !== "string") {
    throw new Error("event_id is required");
  }
  const { objectType } = identity(body);
  const result = await writeProperties(
    body,
    projectionProperties(objectType, body.payload),
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
    if (body.operation === "pull") {
      return response(200, await pull(body));
    }
    if (body.operation === "push") {
      return response(200, await push(body));
    }
    if (body.operation === "upsert") {
      return response(200, await upsert(body));
    }
    return response(400, {
      message: "operation must be health, pull, push, or upsert",
    });
  } catch (error) {
    const status = Number(error?.status);
    return response(
      Number.isInteger(status) && status >= 400 && status < 600 ? status : 400,
      { message: error instanceof Error ? error.message : "sync failed" },
    );
  }
};
