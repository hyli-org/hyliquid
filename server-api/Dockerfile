FROM oven/bun:1-alpine

WORKDIR /app
COPY package.json bun.lock ./
RUN bun install --production

COPY . .
RUN bun run build

EXPOSE 3000
CMD ["bun", "start"]