SECRET_KEY = "benchmark-only"
DEBUG = False
ROOT_URLCONF = "urls"
ALLOWED_HOSTS = ["127.0.0.1", "localhost"]
DATABASES = {"default": {"ENGINE": "django.db.backends.postgresql", "NAME": "djangors_bench", "USER": "bench", "PASSWORD": "bench", "HOST": "127.0.0.1", "PORT": "5432"}}
MIDDLEWARE = []
INSTALLED_APPS = []
