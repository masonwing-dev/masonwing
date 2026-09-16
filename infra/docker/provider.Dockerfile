FROM python@sha256:a9bee15510a364124aa24692899d269835683b883de42f7ebec8c293cf679ccb
RUN mkdir /data /app && chown 10001:10001 /data /app
COPY infra/providers/server.py /app/server.py
USER 10001:10001
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
EXPOSE 8080
CMD ["python3", "/app/server.py"]
