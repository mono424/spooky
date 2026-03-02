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
