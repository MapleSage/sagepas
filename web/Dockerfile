FROM node:22-alpine AS build
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY index.html vite.config.ts tsconfig*.json ./
COPY public ./public
COPY src ./src

ARG VITE_AZURE_CLIENT_ID
ARG VITE_AZURE_TENANT_ID
ARG VITE_API_SCOPE
ARG VITE_REDIRECT_URI
ENV VITE_AZURE_CLIENT_ID=$VITE_AZURE_CLIENT_ID \
    VITE_AZURE_TENANT_ID=$VITE_AZURE_TENANT_ID \
    VITE_API_SCOPE=$VITE_API_SCOPE \
    VITE_REDIRECT_URI=$VITE_REDIRECT_URI
RUN test -n "$VITE_AZURE_CLIENT_ID" \
 && test -n "$VITE_AZURE_TENANT_ID" \
 && test -n "$VITE_API_SCOPE" \
 && test -n "$VITE_REDIRECT_URI" \
 && npm run build

FROM nginxinc/nginx-unprivileged:1.27-alpine
USER root
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
RUN chmod 644 /etc/nginx/conf.d/default.conf && chmod -R a+rX /usr/share/nginx/html
EXPOSE 8080
USER nginx
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1
