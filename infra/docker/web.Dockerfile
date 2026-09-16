# syntax=docker/dockerfile:1
FROM node@sha256:367679cf9792759492a486e4aa4b421764d71a9546a6dae8aab81a99eb797b3e
WORKDIR /workspace
RUN npm install --global pnpm@11.24.0
COPY . .
RUN pnpm install --frozen-lockfile && chown -R node:node /workspace
USER node
EXPOSE 5173
CMD ["pnpm", "dev"]
