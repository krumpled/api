with new_user as (
    insert into krumnet.users
      (default_email, name)
    values
      ($1, $2)
    returning id 
) insert into krumnet.google_accounts
    (email, name, google_id, user_id)
  select
    $3, $4, $5, new_user.id
  from new_user;
