-- A sample for the highlight tests.
CREATE TABLE points (
    id SERIAL PRIMARY KEY,
    label TEXT NOT NULL DEFAULT 'origin',
    x NUMERIC(10, 2)
);

SELECT label, x * 2.5 AS scaled
FROM points
WHERE id > 42 AND label <> 'skip';

this is not ) valid sql at all (;

/* Block comments work too. */
INSERT INTO points (label, x) VALUES ('far', 100);
