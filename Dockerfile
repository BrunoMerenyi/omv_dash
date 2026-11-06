FROM python:3.12-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

# smartctl is only used if you enable SMART inside the container (see README).
RUN apt-get update \
 && apt-get install -y --no-install-recommends smartmontools \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY app ./app

# Unprivileged user; compose.yml adds the docker group for the socket.
RUN useradd --system --uid 10001 --no-create-home dashboard
USER 10001

EXPOSE 6767
HEALTHCHECK --interval=30s --timeout=3s CMD python -c "import urllib.request,os; urllib.request.urlopen(f'http://127.0.0.1:{os.getenv(\"PORT\",\"6767\")}/health', timeout=2)"
CMD ["python", "-m", "app.main"]
