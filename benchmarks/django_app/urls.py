from django.db import connection
from django.http import HttpResponse
from django.urls import path

def hello(request): return HttpResponse("Hello, world!")
def full_stack(request):
    with connection.cursor() as cursor:
        cursor.execute("SELECT id FROM polls_question WHERE pub_date <= NOW() ORDER BY pub_date DESC LIMIT 5")
        rows = cursor.fetchall()
    return HttpResponse("<ul>" + "".join(f'<li><a href="/{row[0]}/">{row[0]}</a></li>' for row in rows) + "</ul>")

urlpatterns = [path("hello/", hello), path("", full_stack)]
