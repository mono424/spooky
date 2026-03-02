DEFINE BUCKET IF NOT EXISTS profile_pictures BACKEND "memory"
  PERMISSIONS WHERE
    $action NOT IN ['put']
    OR (
      file::head($file).size <= 5242880
      AND (
        string::ends_with(file::key($file), '.jpg')
        OR string::ends_with(file::key($file), '.jpeg')
        OR string::ends_with(file::key($file), '.png')
        OR string::ends_with(file::key($file), '.gif')
      )
    );

DEFINE FIELD IF NOT EXISTS profile_picture ON TABLE user TYPE option<string>
PERMISSIONS
    FOR select WHERE true
    FOR create, update WHERE $access = "account" AND id = $auth.id;
