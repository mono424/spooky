# Build context: repo root
# docker build -f example/app-solid/Dockerfile -t example-app .

FROM node:22-alpine AS build

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY example/app-solid/package.json example/app-solid/
COPY packages/ packages/

RUN pnpm install --frozen-lockfile

COPY example/app-solid/ example/app-solid/

ARG VITE_DB_ENDPOINT
ENV VITE_DB_ENDPOINT=$VITE_DB_ENDPOINT

RUN pnpm --filter @example/app-solid build

FROM nginx:alpine

COPY example/app-solid/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/example/app-solid/dist /usr/share/nginx/html

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
