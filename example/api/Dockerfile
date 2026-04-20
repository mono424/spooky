# Build context: repo root
# docker build -f example/api/Dockerfile -t example-api .

FROM node:22-alpine AS build

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY example/api/package.json example/api/

RUN pnpm install --frozen-lockfile

COPY example/api/ example/api/

FROM node:22-alpine

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

COPY --from=build /app /app

WORKDIR /app/example/api

EXPOSE 3660

CMD ["pnpm", "tsx", "src/index.ts"]
